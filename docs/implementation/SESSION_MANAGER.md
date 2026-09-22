# Session Manager実装

## 目的

複数のDiscord Eventを跨ぐ短時間の対話状態をRust Coreで有限に管理する。TypeScript Discord Adapterと個別CommandにはSession保存、期限Timer、Queueを持たせない。

Phase 9の最初の利用者は`o.sale <文字列>`である。Phase 10では`ut`/`tut`の選択、ページ送り、file選択へ拡張した。

## 構成

`crates/kbc-core/src/session.rs`が次を持つ。

- `SessionId`: Core内部の単調増加ID
- `SessionRequest`: owner、channel、TTL、完了時Cleanup方針、Continuation
- `SessionContinuation`: 受付Reaction一覧、非同期の選択後Action生成、任意のtimeout表示
- `SessionManager`: 待機中・有効中Session、容量、期限、Cleanup

Session ManagerはRuntime Workerだけが所有する。Event処理が直列であるためMutexは不要である。

## 状態遷移

```text
Session付きCommandOutput
→ promptのActionIdで待機
→ sendMessage成功(messageIdあり)
→ Reaction Actionを生成し、最後の追加結果を待つ
→ 全Reaction追加完了後にmessageIdで有効化
→ owner・channel・message・emojiが一致
→ Sessionを先に削除
→ 必要なSessionでは全Reactionを削除
→ Continuationを1回実行
→ 終了、同じMessageを再武装、または新しいprompt MessageへSessionを引き継ぐ
```

送信失敗、message IDなし、期限切れではSessionを破棄する。無関係なUser、Message、Channel、ReactionはSessionを消費しない。

## 上限と期限

- 待機中と有効中の合計: 64件
- 1 SessionのReaction: 1〜11件
- TTL上限: 5分
- `sale`選択TTL: 30秒
- Reaction追加完了待ち上限: 2分

prompt結果待ちとReaction追加完了待ちにも期限を適用し、`actionResult`が失われても残留させない。選択用の30秒は最後のReaction追加成功後から開始する。Workerは全Session中の最短期限を待つため、新しいEventが来なくてもCleanupされる。容量超過時はpromptを再実行案内へ置き換え、Reactionのない選択一覧を送らない。

11件はfile選択1ページの数字9件と、前後ページの2件を有限に収めるための上限である。

## 継続方式

- `SessionResume::complete`: Actionを返して終了する
- `SessionResume::continue_with`: 同じDiscord Messageを編集し、Reactionを再設定する
- `SessionResume::start_new`: 最後の`sendMessage` Action結果を新しいSession promptとして登録する

`continue_with`は検索結果とfile選択のページ送りに使う。`start_new`は検索候補を選んだ後、別Messageでfile選択を始める場合に使う。いずれもowner、channel、Request ID、TTLとCleanup方針を引き継ぐ。

ContinuationはHTTP取得を必要とするorigin画像・file存在確認を扱えるよう、所有型を消費する非同期Futureを返す。Runtime WorkerはContinuation完了まで直列処理を維持し、Command固有TaskやQueueを作らない。

## timeout

有効Sessionが期限切れになると、Session Managerは必要に応じて`clearReactions`と`editMessage`を生成する。`ut`/`tut`は現在ページの内容を保ったまま「受付は終了しました」へfooterを変更する。`sale`は旧仕様どおり表示を変更せず、Core内状態だけを解放する。

## `sale` Continuation

`SaleSelection`はReactionとstage IDの対応、channel ID、取得済み`SaleDisplayData`を所有する。選択時に外部HTTPを再実行せず、同じSnapshotから既存の詳細整形処理を呼ぶ。

旧版に合わせ、次を維持する。

- 実行UserのReactionだけを受け付ける
- 30秒で受付終了する
- 選択後は該当IDの詳細を送る
- 選択を受け付けたら全Reactionを削除してから詳細を送る
- タイムアウト時はMessage編集やReaction削除を行わない
- 10件以上では従来どおり一覧だけを送り、Sessionを作らない

## Protocolとの関係

Session IDはCore内部だけで使い、Protocol Version 1は変更しない。promptの`ActionId`と成功結果のDiscord message IDで状態を結び付ける。Reaction追加と選択後のActionは元Commandの`RequestId`を維持する。

Unicode ReactionはAdapterが生のemojiへ正規化する。Custom emojiは従来どおりDiscordのidentifierを使う。

## 最小検証

純粋なSession状態遷移Test 1件で、登録、Reaction追加完了後の有効化、40秒かかるReaction準備、owner制限、選択時Cleanup、1回だけのContinuation実行を確認する。Compiler、Clippy、既存Runtime Smokeと実Discord確認を主な防衛線とする。
