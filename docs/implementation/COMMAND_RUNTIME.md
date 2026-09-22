# CommandRuntime実装

## 実装範囲

Phase 5では、Rust CoreへParser、Registry、Dispatcher、CommandContext、Command traitを追加し、最初のCommandとして`ping`を登録した。TypeScript AdapterとN-API公開面は変更しない。

## Command API

`crates/kbc-core/src/command.rs`がCommand共通APIを持つ。

- `CommandMetadata`: Commandが所有する`name`、`aliases`と`guild_only`
- `CommandContext`: 対象Channel IDと実行User IDを所有する
- `CommandFuture`: 有限な`CommandOutput`を返すSend可能なboxed Future
- `Command`: Metadataとasync `execute()`を要求するtrait
- `CommandRegistry::new(commands)`: nameとaliasをlowercase化してindexを作り、重複をerrorにする
- `CommandRegistry::resolve(name)`: 正規化済みnameからCommand trait objectを返す

CommandはAction envelope、Action ID、Request ID、N-API、discord.jsを知らない。Phase 8以降は1回の実行から最大32件のAction dataを返せる。Phase 9では必要なCommandだけがAction 1件と`SessionRequest`を組にした出力を返せる。

Phase 10ではSession Continuationが同じMessageを再武装するか、最後の`sendMessage`を新しいSession promptとして登録できる。これにより`ut`/`tut`のページ操作とfile選択をCommand Runtime外へ状態流出させず実装する。

Phase 7でMetadataのnameとaliasesを所有型へ変更し、Content File名から静的Commandを構築できるようにした。Allocationは起動時だけであり、実行のHot Pathには追加されない。

## CommandRuntime

`crates/kbc-core/src/command_runtime.rs`がEventからActionまでを調整する。

- `CommandRuntime::new(content)`: Contentから必要な文字列を受け取り、組み込みCommandをRegistryへ登録する
- `parse_command_input(content, prefix)`: Prefixを検査し、Command名をASCII lowercase化して引数を空白分割する
- `CommandRuntime::handle_event(event)`: Message EventだけをParseし、Registry解決、Guild判定、Command実行、Action envelopeと任意のSession登録要求を生成する

Action IDは入力Event IDから`action:{eventId}`として作り、Request IDは入力Eventから維持する。

```text
CoreEvent.messageCreate
→ parse_command_input
→ CommandRegistry.resolve
→ CommandMetadata.guild_only
→ Command.execute
→ CoreActionData
→ CoreAction envelope
```

Prefix外、空Command、未知Command、Guild限定CommandのDM入力では`None`を返す。ReactionとAction ResultはCommandへ渡さず、WorkerがSession Managerへ渡す。

## 静的Command

`crates/kbc-core/src/commands/static_response.rs`が、`content/responses`の5件を共通実装で処理する。

```text
o.<static-command> [任意の引数]
→ responseをsendMessage

o.<static-command> help
→ command helpをsendMessage
```

helpの比較はASCIIの大文字小文字を区別しない。helpがない場合はresponseへfallbackする。Channel IDはCommandContextから取得し、Local表示の`[local] `はDiscord Adapterの責務とする。

## help

`crates/kbc-core/src/commands/help.rs`が`content/help/index.txt`を1件送信する。引数は無視する。Phase 7時点のindexは現行Contentを維持しているため、未移植Commandも記載されている。

## AppRuntimeとの関係

`AppRuntime::start()`がCommandRuntimeを作り、Workerへ所有権を渡す。Registry重複があればRuntime開始を失敗させる。

WorkerはEventを1件ずつCommandRuntimeへ渡し、返されたActionをbounded Action Queueへ送る。Commandは独自Queueやspawnを行わない。

Phase 9以降、WorkerはSession付きCommand出力をSession Managerへ登録してからAction Queueへ送る。Session Managerは元のRequest IDを保持し、Action ResultとReactionをCommand Runtimeの外で処理する。Session継続処理の詳細は`docs/implementation/SESSION_MANAGER.md`を参照する。

## 最小Smoke

既存のRuntime Smoke 1系統を次へ更新した。

```text
o.HOME ignored → 静的Response
o.PING HELP    → 静的Command help
o.help ignored → help index
```

個別CommandのUnit Testは追加しない。Compilerとこの境界Smokeを主な防衛線とする。

## Phase 5検証結果

- Rust compiler、Clippy、rustfmt、TypeScript typecheckが成功した。
- DebugとReleaseのNative buildおよびRuntime Smokeが成功した。
- 実DiscordのGuild Channelで`o.PING ignored`を受信し、Local識別付きの`[local] pong!!`を返信した。
- 応答後にerrorがなく、`SIGINT`でAdapter停止完了まで到達した。
- Discord TokenはV2へ保存せず、検証Processの環境変数としてだけ利用した。

## Phase 7検証結果

- DebugとReleaseのRuntime Smokeで静的通常返信、個別help、help indexが成功した。
- 実Discordで`o.ping`、`o.home`、`o.ping help`、`o.help`のLocal返信を確認した。
- 静的Commandとhelpはいずれも1実行1 Actionで完結し、Command APIへ追加のRuntime機構を必要としなかった。
- Local起動・停止をnpm Script経由で確認した。

## 後続Phaseへ残すこと

- 複数Actionが必要になった時点でbounded emitterを設計する
- Authorization対象CommandでUser・Role capabilityをCommandContextへ追加する
- Service導入時に必要なCapabilityだけをCommandへ渡す
- Button等の新しいInteractive要件が出た場合、実利用者に必要な入力だけをSession Managerへ追加する
