---
layout: default
title: Firecracker セットアップ（Tier 3）
nav_order: 8
nav_exclude: true
---

# Firecracker セットアップ（Tier 3）

Firecracker は、独自のゲストカーネルを持つ microVM 内でワークロードを実行します。
ホストカーネル、VMM、ゲストイメージ、運用者の設定は引き続き信頼境界に含まれます。
この分離は、あらゆる脱出経路を防げるという保証ではありません。

Tier 3 はオープンソースランタイムの一部です。ライセンスキーも Enterprise ビルドも必要ありません。`symbi-sandbox-guest` と `symbi-sandbox-supervisor` のクレートはこのリポジトリに含まれているため、ゲストイメージを自分でビルドし、監査し、再現できます。

最新の設定手順は [英語版ガイド](../firecracker-setup.md) を参照してください。
対応する `symbi-sandbox-guest`、カーネル、rootfs と、制限付き vsock プロトコルを説明しています。
ホストの作業ディレクトリは VM に自動転送されません。ツールの結果はシリアルコンソールから取得しません。

通常のスーパーバイザーはユーザーアカウントで動作し、ゲストの CPU とメモリを
[共有プール](../shared-budgets.md) に予約します。jailer やホスト cgroup は構成せず、
VMM の追加メモリも予約しません。

オプションの [管理ホストサービス](../firecracker-host-service.md) は、承認済みアーティファクト、
jailer、VMM ごとの専用 ID、ホスト cgroup、VMM の追加分を含むメモリ予約を提供します。
systemd がサービスとクリーンアップを監督します。`service_uid = 0` はこのサービスを必須とし、
利用できない場合にローカルの代替プロセスを起動しません。Docker/gVisor の容量は別に割り当てる必要があります。

ビルド、対象を絞ったテスト、Docker の回帰 E2E は成功しています。
KVM、サービス障害、watchdog を含む特権ホスト E2E は未完了です。
このプロファイルを検証済みとするには、配備先ホストでの試験が必要です。
[ホストサービスの手順](../firecracker-host-service.md) に構築と試験のコマンドがあります。
