# 統合設定UI仕様

## 目的

引数の多い`o.push`と`o.maint`を、Discordのボタン・選択メニュー・Modalを使う`o.settings`へ段階的に移行する。Slash Commandは他Botとの名前競合を避けるため使用しない。

旧`o.push`と`o.maint`は移行期間中も同じStorage・通知ロールServiceを使って動作させる。新UIへの移植と実Discord確認が完了した後に削除する。

## 起動と権限

- `o.settings`はGuild限定とする。
- 固定Bot管理者または登録済みBotメンテナーだけが実行できる。
- Commandは公開Messageへ「設定を開く」ボタンを表示する。
- 起動ボタンはCommand実行者だけが使用できる。
- ボタン以降の設定メニューと保存結果はephemeralで表示する。
- Interactionごとに最新の権限を再検査する。
- Botメンテナーの追加・削除は固定Bot管理者だけが実行できる。

## 画面構成

設定メニューは次の3分類とする。

1. 通知設定
2. 通知ロール共通設定
3. Botメンテナー設定

設定対象の選択はMessage Componentで行い、選択済み対象の最終編集と確認だけをModalで行う。Modalの最大5 Componentへ全設定を詰め込まず、各送信を独立して保存可能な1変更とする。

## 通知設定

通知種別の表示名は次とする。

- スケジュール更新通知
- `notice`更新通知
- `ad`更新通知
- Android版更新通知
- iOS版更新通知

通知設定の識別単位は現行どおり「Guild・通知種別・Channel」とする。同じ通知種別を複数Channelへ登録でき、Channelごとに異なるmention Roleを設定できる。

- 登録済みChannelは5件ずつ表示し、前後へページ切り替えできる。
- `o.settings`を実行したChannelを「このチャンネルを設定」から選べる。
- DiscordのChannel Selectから別Channelを選べる。
- 保存前にAdapterでBotが送信可能なGuild Channelか確認する。
- Modalでは設定保存または通知先解除と、登録済み通知Roleからmention対象を選ぶ。
- Modal送信1回につきGuild設定を1回だけ更新する。

スケジュール関連サイトURLはChannel別ではなくGuild共通である。スケジュール更新通知内の独立したModalで、1行1URL、最大9件として一覧をまとめて置き換える。空欄は全削除とする。

## 通知ロール共通設定

- 既存RoleをRole Selectから登録できる。
- 権限を持つRole、managed Role、Botが管理できないRoleは拒否する。
- 新規通知Roleは名前をModalへ入力し、権限ゼロで作成後そのまま通知Role一覧へ登録する。
- 通知Role削除時は、全通知先のmention設定からも同じRole IDを削除する。
- 既にMemberへ付与済みのRoleは一括解除しない。
- Role変更後は設置済みの選択パネルを更新する。
- 現在ChannelまたはChannel Selectで選んだ別Channelへ単一の選択パネルを設置・移動できる。
- パネル移動時の旧パネル無効化順序は現行仕様を維持する。

## Botメンテナー設定

- 設定はGuild単位ではなくBot全体で共通とする。
- 固定Bot管理者はUser SelectまたはRole Selectから対象を追加できる。
- 登録済みIDは20件ずつ表示し、選択メニューから削除できる。
- 通常のBotメンテナーには現在Guildで該当するMember一覧だけを最大25人表示する。
- 更新は既存の最大64件、SHA付きWrite、競合時1回再試行、応答消失時の再読込確認を維持する。

## Interaction応答

Message ComponentはModal表示またはMessage更新を初回応答として行う。StorageやGuild Member読込みを伴う一覧画面はAdapterが即時にdefer updateし、取得完了後に同じephemeral Messageを編集する。Modalを開く操作の権限と初期値は待機しないStorage snapshotから読み、別の保存処理中なら再試行を案内する。Storage書込みや複数Discord操作を伴うModal送信はAdapterが即時にephemeral deferし、Core処理完了後に結果を編集する。

Adapterが保持する未完了Interaction参照は最大64件、15分TTLとする。設定状態や権限判断は保持せず、Discord Interactionへ応答するための一時的なTransport参照だけを管理する。

## 実Discord確認項目

- `o.settings`の入口Buttonと、Command実行者以外の操作拒否
- 5種類の通知先の追加・更新・解除、複数Channel、ページ切り替え
- 実行ChannelとChannel Select、送信権限不足Channelの拒否
- スケジュール関連URLの置換と全削除
- 通知Roleの既存Role登録、新規作成と自動登録、削除、選択パネル移動
- BotメンテナーのUser・Role追加、削除、固定Bot管理者以外の変更拒否
- 移行期間中の`o.push`と`o.maint`の既存動作
