# Interaction設定UI V1

## 状態

採用。

## 背景

通知・メンテナー設定は`o.push`と`o.maint`の複数階層の引数へ分散しており、ID、`add`、`del`、`off`を手入力する必要があった。Discord固有UIを利用して入力ミスを減らしつつ、権限・検証・Storage更新はRust Coreへ維持する必要がある。

Slash Commandは他BotとのCommand名競合を避けるため採用しない。Prefix CommandからModalを直接開くことはできないため、`o.settings`がButton付きMessageを送り、Button Interactionからephemeral設定メニューを開始する。

## 判断

KBC Protocolへ汎用的なMessage Component、Modal、Interaction Event・Actionを追加する。設定固有のDiscord操作をTypeScriptへ実装しない。

```text
o.settings messageCreate
→ sendInteractiveMessage
→ Button Interaction
→ componentInteraction
→ replyInteraction(ephemeral)
→ 選択メニュー・Buttonで対象選択
→ showModal
→ AdapterがModal送信をephemeral defer
→ modalSubmit
→ Rust Coreで検証・保存
→ editInteractionReply
```

選択メニューは設定対象の選択に使い、Modalは選択済み対象の最終編集に限定する。Modalを跨ぐ下書きSessionは持たず、1回のModal送信を独立して保存可能な変更とする。

## 責務

### TypeScript Discord Adapter

- `interactionCreate`の受信
- Discord Interactionからplain Protocol dataへの変換
- Button、Select、ModalのDiscord object構築
- Modal送信の即時defer
- 最大64件・15分TTLのInteraction応答参照
- Channel・Role等のDiscord固有検査

### Rust Core

- 設定画面構成
- custom IDと入力値の検証
- 権限判定
- 現在値の読込み
- Storage更新
- ページ切り替え
- ユーザー向け結果文

AdapterのInteraction参照はDiscord応答用Transport stateであり、設定SessionやDomain stateを所有しない。

Coreは設定Interactionを最大4件の管理対象Taskとして処理する。外部Storage WriteやGuild照会を待つInteractionがあっても別のModal表示を止めず、上限到達時は新規操作へ混雑案内を返す。

## Storage更新

通知先ModalはSubscriptionの有効状態とRole一覧をまとめて更新する。Roleを1件ずつ書き換えて複数回GitHubへ保存しない。関連サイトURLもModalの一覧を1回のGuild設定更新で置き換える。

旧`o.push`と`o.maint`は移行期間中維持する。通知Role・選択パネル操作は共通Serviceへ切り出し、新旧UIから再利用する。

## トレードオフ

- Prefix Command起点のため、最初の起動Buttonだけは公開Messageになる。
- Message ComponentからModalを開くまでDiscordの初回応答期限があるため、Modal定義に必要な値は起動時復元済みのStorage snapshotから読む。
- Storage・Guild Member読込みを伴う一覧画面のComponentは先にdefer updateし、取得後に同じephemeral Messageを編集する。
- Modalを開くComponentはdeferできないため、権限とModal初期値は待機しないStorage snapshotから読む。保存処理がStorageを使用中なら即時に再試行を案内する。
- Modal送信はdeferできるため、GitHub書込みと複数Discord操作を完了まで待てる。
- Interaction参照Mapは追加されるが、件数とTTLを固定し、設定や権限を保持しない。
