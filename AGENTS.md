# KBC Discord Bot 開発ルール

このリポジトリは、長期運用・拡張性・堅牢性・限られたリソースでの高速動作を重視する。

作業前に変更内容に応じて以下を読むこと。

- 設計原則: `docs/architecture/PRINCIPLES.md`
- Runtime設計: `docs/architecture/RUNTIME.md`
- TypeScript ↔ Rust境界: `docs/architecture/PROTOCOL.md`
- Discord Adapter設計: `docs/architecture/DISCORD_ADAPTER.md`
- Command Runtime実装: `docs/implementation/COMMAND_RUNTIME.md`
- Content Catalog実装: `docs/implementation/CONTENT.md`
- 外部Data Command実装: `docs/implementation/EVENT_DATA_COMMANDS.md`
- HTTP・複数Action判断: `docs/decisions/HTTP_AND_MULTI_ACTION_V1.md`
- Local開発起動: `docs/implementation/LOCAL_DEVELOPMENT.md`
- Command追加・変更: `docs/engineering/COMMANDS.md`
- 性能・負荷設計: `docs/engineering/PERFORMANCE.md`
- テスト方針: `docs/engineering/TESTING.md`
- Rust Core V2計画: `docs/plans/RUST_CORE_V2.md`

## 最重要原則

- TypeScriptは`discord.js`を扱う薄いDiscord Adapterとする。
- Discordに依存しないBotロジックは原則Rust Coreへ置く。
- TypeScriptとRustはKBC Protocolを介して接続する。
- Commandごとに独自のHTTP・Cache・Queue・Timeout・Progress基盤を作らない。
- 無制限の並列処理・Queue・Cache・Session・Bufferを作らない。
- 性能改善は推測ではなく計測結果に基づく。
- テストは超最小限とし、型・コンパイラ・設計で保証できることを重複してテストしない。
- 現行仕様や設計判断を確認せず既存挙動を変更しない。
- 関係のない大規模リファクタリングを同時に行わない。
- より良い設計を発見した場合は、実装前に理由とトレードオフを示す。

「動いた」だけでは完了としないが、「念のため」だけを理由に大量の仕組みやテストを追加しない。
