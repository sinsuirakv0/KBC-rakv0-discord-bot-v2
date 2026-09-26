# ストア版アップデート通知

## 利用方法

AndroidとiOSは通知先・メンションRole・検知状態を分離する。

```text
o.push update android
o.push update android off
o.push update android role:<ロールID>
o.push update android role:<ロールID> del

o.push update ios
o.push update ios off
o.push update ios role:<ロールID>
o.push update ios role:<ロールID> del
```

すべて固定Bot管理者またはBotメンテナーだけが実行できる。Roleは先に`o.maint pushsetting <ロールID> add`で通知用Roleへ登録する。

## 監視設計

Rust Coreの`NotificationService`がAndroidとiOSの監視Loopを1本ずつ所有する。対象PlatformのSubscriptionが存在する場合だけ、両Loopが互いを待たず5秒間隔で照会する。失敗時は15秒、60秒、300秒へ有限にbackoffし、成功後は5秒へ戻す。停止SignalはSleep、Action Queue待ち、Action結果待ちのすべてを中断する。

- Android: Google Play公開Pageから起動時に`ds:5`のRPC情報を取得し、以後は小さい`batchexecute`応答から公開`versionName`を読む。RPCまたはschemaが変わった場合は公開Pageを再取得して1回だけ再試行する。
- iOS: Apple公式iTunes Search APIの日本向けLookupから`version`を読む。各照会へcache-busterを付ける。5秒間隔はAppleの案内する概ね20回/分以内の12回/分である。

候補Versionを見つけた時は同じSourceを即時にもう一度読み、一致した場合だけ確定する。`15.6.0`から`15.6.1`のような数値Segmentの増加だけを通知し、一時的な巻き戻りは通知しない。Androidだけ、またはiOSだけの更新は該当Platformだけへ通知する。

## 永続化と配送

最初の観測値は通知せず、private Data Repositoryの`state/store-versions.json`へ基準値として保存する。AndroidとiOSのVersionは別Fieldである。通知Event IDもPlatformとVersionから別々に作る。

更新時は既存Notification Runtimeを利用し、Event Record、送信前の`attempting`、Discord Action結果、`sent`を保存する。通知が成功してからだけ基準Versionを進めるため、送信失敗時は次回照会で同じ冪等Eventを再処理できる。`attempting`の結果が不明な場合は既存仕様どおり自動再投稿せず、調停を要求する。

通知本文は次の形式とする。Androidでは2行目の`ios`を`android`へ、`App Store`とURLを`Google Play`とGoogle Play URLへ置き換える。KBC差分URLの`version`と`compare`は各Versionを`15.7.0 → 150700`の形式へ変換する。

```text
NEW Version
ios Ver.15.7.0
検知時刻: 2026/09/26(土) 12:34:56
KBC
https://kbc-rakv0.vercel.app/pages/asset-explorer/?dataset=Local&version=150700&compare=150600&view=diff&layout=grid&offset=200
App Store
https://apps.apple.com/jp/app/id547145938
※反映まで少し時間がかかります
```

## 既知の境界

Google Playには他社Appの公開Versionを取得する公式APIがないため、公開Pageが利用するRPC schemaへ依存する。schema変更はbackoff付きErrorとして扱い、誤ったVersionを通知しない。Apple側もStoreの公開反映より先に検知することはできない。したがって「配信後できるだけ早く」は満たすが、Store内部で公開される前の検知は保証しない。
