# Botメンテナー仕様

## 権限モデル

- 固定Bot管理者3人のIDと権限は変更しない。
- 固定Bot管理者だけが`o.maint maintainer`でメンテナー対象を追加・削除・確認できる。
- メンテナー設定はGuild単位ではなく、Bot全体で共通の設定とする。
- 登録値はDiscordユーザーIDまたはDiscordロールIDであり、Discord上に専用ロールを新設しない。
- Command実行者のユーザーID、または所持ロールIDのいずれかが登録値と一致すればメンテナーとして扱う。
- 現在メンテナー権限を使う管理Commandは`o.push`である。通常の公開Commandは制限しない。

## Command

```text
o.maint maintainer <ユーザーIDまたはロールID>
o.maint maintainer <ユーザーIDまたはロールID> del
o.maint maintainer list
```

ユーザー・ロールのmention表記も受け付け、内部ではSnowflake文字列へ正規化する。登録対象は最大64件とし、重複追加と未登録対象の削除は冪等に成功する。

`list`は実行したGuildに現在所属し、登録ユーザーIDと一致するか、登録ロールを所持するBot以外のユーザーを表示する。同じユーザーが複数条件へ一致しても1回だけ表示する。一覧は名前順で最大25人まで表示する。

## 永続化

private GitHub Data Repositoryの`config/maintainers.json`を正本とする。

```json
{"schemaVersion":1,"subjectIds":["123456789012345678"]}
```

更新は既存Storageと同じSHA付きWrite、競合時1回再試行、応答消失時の再読込確認を使う。Guildごとの設定ファイルには保存しない。

## Discord要件

ロールに所属するユーザーを完全に列挙するため、Bot ApplicationでServer Members Intentを有効にし、Adapterでも`GuildMembers` Intentを要求する。メンバー全取得は5,000人以下のGuildに限定し、それを超える場合は一覧取得を失敗させる。権限判定自体は`messageCreate`に含めた実行者のRole IDだけで行うため、一覧取得の成否には依存しない。
