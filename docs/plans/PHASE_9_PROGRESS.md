# Phase 9 作業チェックポイント

- 状態: 完了
- 最終更新: 2026-09-21
- 対象: Session Managerと`sale` Reaction選択

## 確定した方針

- Session状態はRust CoreのRuntime Workerが所有し、TypeScript Discord AdapterへCommand固有状態を置かない。
- prompt送信前はAction ID、送信成功後はDiscord message IDでSessionを識別する。
- Session IDはCore内部だけに置き、KBC Protocol Version 1は変更しない。
- 元CommandのRequest IDをReaction追加・選択後の詳細返信まで維持する。
- Sessionは最大64件、Reactionは最大9件、TTLは最大5分に固定する。
- `sale`の30秒は旧版と同じく、全Reaction追加完了後から開始する。
- 選択成立時は全Reactionを削除してから詳細を返す。タイムアウト時は削除しない。
- Eventが来ない場合も最短期限TimerでCleanupする。
- `sale`は旧版どおり実行Userのみ、30秒、タイムアウト表示変更なしとする。
- 選択時は取得済みSale dataを再利用し、HTTPを再実行しない。

## 作業状況

- [x] 旧`sale` Reaction選択仕様の調査
- [x] `discord.js` Unicode Reaction表現の調査
- [x] Session ID・Owner・Expiry・Continuation・Cleanup
- [x] 待機中Actionと送信Message IDの関連付け
- [x] Session容量・Reaction数・TTL上限
- [x] `sale` 1〜9件のReaction選択
- [x] Unicode emojiのProtocol正規化
- [x] Reaction追加が30秒を超える場合のSession期限修正
- [x] 選択成立時の全Reaction Cleanup
- [x] Unit Test・Compiler・Clippy・TypeScript型検査
- [x] 設計・実装Document更新
- [x] Runtime Smoke再確認
- [x] Docker image再buildとContainer Runtime Smoke
- [x] 実Discordで`sale` Reaction選択確認
- [x] BOM・秘密情報・旧Repository非変更の最終監査

## 検証結果

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p kbc-core`（6件成功）
- `npm run typecheck`
- `npm run smoke:runtime`
- `docker build --progress=plain -t kbc-rakv0-discord-bot-v2:local .`
- Linux Container内の`Runtime smoke passed.`
- 実Discordで一覧、全Reaction追加、選択、全Reaction削除、詳細返信を確認
- Local Bot Processが残っていないことを確認
- Token値がV2 `.env`以外に重複していないことを確認
- 変更した日本語FileがBOM付きUTF-8であることを確認

## 中断時の再開地点

Phase 9は完了した。Session Managerと`sale` Reaction選択を実装し、初回の実Discord確認で発見した「Reaction追加中にも30秒を消費する」問題も修正した。最後のReaction追加結果から30秒を開始し、選択成立時は全Reactionを削除してから詳細を返す。

自動検証、Linux Container smoke、実Discord確認、最終監査はすべて成功した。Local Botと検証Containerは停止済みである。

次の作業はPhase 10である。Task Runtimeを先に一般化せず、旧版の長時間・CPU-heavy・外部Process Commandを調査し、最初の実利用者と必要なQueue、Timeout、Cancellation、Progressの最小要件を確定する。

旧Repositoryには変更を加えていない。Local Tokenは`.env`以外へ書き出していない。
