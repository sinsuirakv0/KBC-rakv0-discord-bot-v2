# ReleaseとGitHub Packages

## Version

Repositoryの公開Versionは`v0.1.0`形式のGit tagとGitHub Releaseで表す。

- patch: 不具合修正や互換性を維持した内部改善
- minor: 新機能や利用者から見える変更
- major: 安定版以降の互換性を壊す変更

Rust workspace、root package、Discord AdapterのVersionは同じ値に揃える。

## Release手順

1. `main`のCI成功を確認する。
2. `Cargo.toml`、`package.json`、`apps/discord/package.json`のVersionを確認する。
3. `vX.Y.Z` tagを対象にGitHub Releaseを公開する。
4. `Publish container` Workflowの成功を確認する。

Release公開時、WorkflowはRelease tagのsourceからDocker imageをbuildし、次のtagでGitHub Container Registryへ公開する。

```text
ghcr.io/sinsuirakv0/kbc-rakv0-discord-bot-v2:X.Y.Z
ghcr.io/sinsuirakv0/kbc-rakv0-discord-bot-v2:latest
```

過去Releaseを初回だけ公開する場合は、`workflow_dispatch`の`ref`へ`vX.Y.Z`を指定する。Workflow定義は`main`から実行されるが、imageのbuild contextには指定tagのsourceを使う。

## PackageとRepositoryの関連付け

Container imageへ`org.opencontainers.image.source` labelを付け、PackageをこのRepositoryへ関連付ける。WorkflowはRepositoryの`GITHUB_TOKEN`だけを使い、外部Registry Tokenを追加しない。

Packageの権限はRepositoryから継承する。公開RepositoryのPackageとして匿名pull可能な状態を維持する。
