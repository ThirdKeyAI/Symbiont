---
layout: default
title: Firecracker セットアップ（Tier 3）
nav_order: 8
nav_exclude: true
---

# Firecracker セットアップ（Tier 3）

Symbiont の Tier 3 サンドボックスは、各エージェントを専用の Firecracker microVM 内で実行します。VM には実際の Linux カーネルと、運用者が用意したルートファイルシステムを使用します。これはランタイムが提供する最強の分離ティアです — エージェントコードはホストカーネルに触れず、他のテナントとカーネル表面を共有することもなく、すべての実行の終了時に破棄されます。

このガイドでは、`symbi init --sandbox tier3` を指す前に何を用意する必要があるかを説明します。

---

## なぜ運用者が用意するのか？

Firecracker は Docker や gVisor とは根本的に異なります。共有カーネル名前空間の中でプロセスを実行するのではなく、提供された実際のカーネルとディスクイメージを起動します。つまり、何がインストールされるか、init がどう実行されるか、ワークロードがどのように届けられるかという **userspace コントラクトを制御するのはあなた** です。信頼の前提、パフォーマンス予算、ランタイム依存関係はデプロイメントごとに異なるため、Symbiont は万能の rootfs を組み込めません。

ランタイム側は完全に実装済みです: `FirecrackerRunner` が実行ごとの VM 設定を書き出し、コードをホスト側の作業ディレクトリに配置し、`firecracker --no-api --config-file <path>` を exec し、シリアルコンソールから出力をキャプチャします。あなたが用意するのは、その作業ディレクトリの利用方法を理解しているカーネル + rootfs です。

---

## 前提条件

ホスト上に3つの成果物が必要です：

1. **`firecracker` バイナリ** — [firecracker-microvm/firecracker releases](https://github.com/firecracker-microvm/firecracker/releases) からインストールします。`symbi doctor` は `$PATH` 上で到達可能かを報告します。
2. **ブートカーネルイメージ（`vmlinux`）** — 圧縮されていない ELF（bzImage ではありません）。プリビルドの CI 成果物をダウンロードする（最速）か、Firecracker 推奨の設定で upstream Linux からビルドします（ラップトップで約10分）。
3. **ルートファイルシステムイメージ（`rootfs.ext4`）** — 小さな Linux userland（Alpine、BusyBox、または Debian）と、Symbiont の VM 内コントラクト（後述）を実装する init スクリプト。

KVM もホスト上で利用可能である必要があります（`/dev/kvm` が `symbi up` を実行するユーザーから読み取り可能）。

---

## クイックスタートレシピ

このレシピは動作する約 30 MB の `rootfs.ext4` を作成し、Firecracker の CI 成果物からプリビルドの `vmlinux` を取得します。意図的に最小限です — ネットワーク、Python、追加のランタイムなし。ワークロードに合わせて適応させてください。

### 1. プリビルドカーネルをダウンロード

```bash
ARCH="$(uname -m)"
curl -fsSL "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${ARCH}/vmlinux-5.10.210" \
  -o /var/lib/firecracker/vmlinux
```

これは Firecracker の CI がテストに使用するカーネルです — ランタイムに対する known-good の組み合わせです。

### 2. 最小限の Alpine rootfs をビルド

```bash
sudo apt-get install -y debootstrap   # or use apk on Alpine hosts

# 256 MB image — size to your workload's working set + scratch space
dd if=/dev/zero of=/var/lib/firecracker/rootfs.ext4 bs=1M count=256
mkfs.ext4 -F /var/lib/firecracker/rootfs.ext4

mkdir -p /mnt/rootfs
sudo mount -o loop /var/lib/firecracker/rootfs.ext4 /mnt/rootfs

# Bootstrap Alpine into the image
sudo apk -X http://dl-cdn.alpinelinux.org/alpine/latest-stable/main \
  -U --allow-untrusted --root /mnt/rootfs --initdb \
  add alpine-base busybox openrc
```

### 3. Symbiont の VM 内 init コントラクトをインストール

microVM 内で PID 1 として実行される init スクリプトは、以下のプロトコルを実装する必要があります。これを `/mnt/rootfs/sbin/symbi-init` として保存します：

```sh
#!/bin/sh
# Symbiont in-VM init — minimal example.
#
# Contract:
#   /work/code      — the agent payload to execute (mode 0755).
#   /work/env.json  — JSON object of env vars the host wants exposed.
#   stdout/stderr   — written to the serial console (ttyS0).
#   exit            — `reboot -f` triggers Firecracker shutdown via panic=1.

mount -t proc  proc  /proc
mount -t sysfs sys   /sys
mount -t devtmpfs dev /dev
mount -t tmpfs tmp /tmp

# Bring up the serial console for stdout capture.
exec </dev/console >/dev/console 2>&1

# /work is mounted by the host (e.g. via a vsock-backed share or a second
# block device — both wiring patterns are valid). Wait briefly for it to
# appear, then run the payload.
for _ in 1 2 3 4 5; do
  [ -x /work/code ] && break
  sleep 0.2
done

if [ ! -x /work/code ]; then
  echo "symbi-init: /work/code not found — host failed to deliver payload" >&2
  reboot -f
fi

# Export env from /work/env.json (one var per line: KEY=VALUE).
if [ -r /work/env.json ]; then
  # Tiny JSON-to-shell-env extractor; replace with `jq` if present.
  awk 'BEGIN{RS=","; FS=":"} /"/{
    gsub(/[ \t"{}\n]/,""); print $1"="$2
  }' /work/env.json | while IFS= read -r line; do
    [ -n "$line" ] && export "$line"
  done
fi

/work/code
RC=$?
echo "symbi-init: exit=$RC"
reboot -f
```

```bash
sudo cp symbi-init /mnt/rootfs/sbin/symbi-init
sudo chmod +x /mnt/rootfs/sbin/symbi-init

# Tell the kernel to run it as init.
sudo ln -sf /sbin/symbi-init /mnt/rootfs/init
```

### 4. アンマウントして検証

```bash
sudo umount /mnt/rootfs
e2fsck -f /var/lib/firecracker/rootfs.ext4
```

### 5. Symbiont プロジェクトに組み込む

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/firecracker/vmlinux \
  --firecracker-rootfs /var/lib/firecracker/rootfs.ext4
```

`symbi init` はスキャフォールディング前に両方のパスが存在することを検証します。生成された `symbiont.toml` は次のようになります：

```toml
[sandbox]
tier = "tier3"

[sandbox.firecracker]
kernel_image_path = "/var/lib/firecracker/vmlinux"
rootfs_path       = "/var/lib/firecracker/rootfs.ext4"
vcpus             = 1
mem_mib           = 512
rootfs_read_only  = true
```

その後、`symbi up` は各エージェントを独自の microVM 内で実行します。

---

## VM 内 init コントラクト

これは rootfs が実装しなければならない内容です。このコントラクトのランタイム側は固定であり、userspace 側はあなたの担当です。

| ホストが書き込むもの | 場所 | フォーマット |
|----------------------|-------|--------|
| エージェントコード | ホスト作業ディレクトリ → VM 内で `/work/code` としてマウント | 実行可能ファイル |
| 環境変数 | ホスト作業ディレクトリ → `/work/env.json` としてマウント | JSON オブジェクト |

| VM が行うべきこと |
|---------------------|
| `/proc`、`/sys`、`/dev`、`/tmp` をマウント |
| `ttyS0` をコンソールとして起動（Firecracker のシリアル出力 → ホストの stdout） |
| `/work/code` を見つけて実行 |
| 終了コードをキャプチャし、コンソールに書き込む |
| VM を停止（`reboot -f` + `panic=1` ブート引数 = シャットダウン） |

VM 終了後、ランタイムはホスト作業ディレクトリをクリーンアップします。

### `/work` のトランスポート選択

上記の例では `/work` をプレースホルダのマウントとして残しています。妥当な配線パターンが2つあり、どちらを選ぶかはカーネル設定によります：

- **vsock** — ホストが vsock CID で待ち受け、init が接続して tarball を受信し、`/work` に展開します。よりクリーンで二次ドライブ不要ですが、`CONFIG_VSOCKETS=y` と rootfs に userspace の vsock クライアントが必要です。
- **二次ブロックデバイス** — ホスト作業ディレクトリを `/dev/vdb` として接続される ext4 イメージとして渡し、init が `/work` にマウントします。カーネル設定はシンプルですが、ホスト側で実行ごとにイメージを作成するステップが追加されます。

Symbiont の `FirecrackerRunner` は現在、作業ディレクトリをホストファイルシステムの `$TMPDIR/symbi-fc-<uuid>/` 以下に書き出し、それを VM 内で表面化する方法を init が知っていることを期待します。カーネルに合うトランスポートを選び、`symbi-init` を適宜更新してください。

---

## ハードニングチェックリスト

本番トラフィックでこれを有効にする前に：

- [ ] **カーネル設定** — 不要なものは削ります。Firecracker は `microvm-kernel-x86_64.config` でビルドしたゲストカーネルを推奨しています。
- [ ] **rootfs を読み取り専用に** — `[sandbox.firecracker]` で `rootfs_read_only = true` を維持します。スクラッチ領域が必要なエージェントは `/tmp`（tmpfs）に書き込みます。
- [ ] **SSH なし、getty なし** — エントリポイントは init スクリプトのみであるべきです。
- [ ] **権限を落とす** — `firecracker` 自体は `jailer` を使用して専用の非特権ユーザー下で実行します。`firecracker_binary` をラッパースクリプトに設定すれば、ランタイムがそれを使用します。
- [ ] **メモリ予算** — `mem_mib` をワークロードが許容する最小値に設定します。低いほどブートが速くなります。
- [ ] **ネットワーク** — 外向きネットワークが不要なエージェントには TAP デバイスを与えるべきではありません。Symbiont は自動作成しません。

---

## トラブルシューティング

**`Firecracker is not available at 'firecracker'`** — バイナリをインストールするか、`[sandbox.firecracker]` の `firecracker_binary` にパスを設定します。

**`Firecracker kernel image not found at ...`** — `kernel_image_path` のパスが存在しません。`symbi init` はスキャフォールド時に検証するため、ファイルを移動した場合を除き init 後にこれが発生することはありません。

**VM はブートするがエージェントがハングする** — init スクリプトが `/work/code` を見つけられていない可能性があります。シリアルコンソールに接続して VM 内のログを確認してください。リファレンスの init には1秒の待機ループがあります。作業ディレクトリのトランスポートが遅い場合は延長してください。

**`/dev/kvm` permission denied** — `symbi up` を実行するユーザーに読み取りアクセスが必要です。`kvm` グループに追加します: `sudo usermod -aG kvm $USER`、その後再ログインしてください。

---

## Symbiont が同梱しないもの

設計上：

- **rootfs ビルダーなし。** 運用者の領域です — rootfs の構築はデプロイメントごとに大きく異なるため、万能のスクリプトは提供できません。
- **カーネルビルドスクリプトなし。** Firecracker プロジェクトはすでにプリビルドカーネルと推奨設定を公開しているため、ベンダリングするのではなくそれにリンクしています。
- **ネットワーク配線なし。** TAP セットアップ、NAT、egress ポリシー — すべてデプロイメントごとです。必要であれば VM JSON 設定の `network-interfaces` で配線します。
- **rootfs に秘密を入れる経路なし。** 秘密はイメージに焼き込むのではなく、`env.json` を介して渡します（`env.json` は実行期間中だけホスト作業ディレクトリに存在します）。

ほとんどのワークロードでは Tier 1（Docker）または Tier 2（gVisor）が正しい選択です。カーネル境界が特に必要な場合 — マルチテナントの信頼されないコード、規制対象データ、または gVisor の syscall フィルタの粒度では不十分なワークロード — に Tier 3 を選んでください。
