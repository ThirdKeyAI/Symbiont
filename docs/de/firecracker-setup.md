---
layout: default
title: Firecracker-Setup (Stufe 3)
nav_order: 8
nav_exclude: true
---

# Firecracker-Setup (Stufe 3)

Die Stufe-3-Sandbox von Symbiont laeuft pro Agent in einer dedizierten Firecracker-microVM mit echtem Linux-Kernel und einem vom Betreiber bereitgestellten Root-Dateisystem. Dies ist die staerkste Isolationsstufe, die das Runtime ausliefert — Agent-Code beruehrt niemals den Host-Kernel, teilt keine Kernel-Oberflaeche mit anderen Tenants und wird am Ende jeder Ausfuehrung zerstoert.

Dieser Leitfaden behandelt, was Sie produzieren muessen, bevor Sie `symbi init --sandbox tier3` darauf ausrichten.

---

## Warum vom Betreiber bereitgestellt?

Firecracker unterscheidet sich grundlegend von Docker oder gVisor. Es bootet einen echten Kernel gegen ein Disk-Image, das Sie bereitstellen, anstatt einen Prozess in einem geteilten Kernel-Namespace auszufuehren. Das bedeutet, **Sie** kontrollieren den Userspace-Kontrakt — was installiert ist, wie init laeuft, wie der Workload ausgeliefert wird. Symbiont kann kein One-size-fits-all-rootfs einbacken, weil sich Vertrauensannahmen, Performance-Budget und Laufzeitabhaengigkeiten je nach Deployment unterscheiden.

Die Runtime-Seite ist vollstaendig implementiert: `FirecrackerRunner` schreibt eine VM-Konfiguration pro Ausfuehrung, legt Ihren Code in ein hostseitiges Arbeitsverzeichnis ab, fuehrt `firecracker --no-api --config-file <path>` aus und erfasst die Ausgabe von der seriellen Konsole. Was Sie mitbringen, ist der Kernel + rootfs, das weiss, wie das Arbeitsverzeichnis zu konsumieren ist.

---

## Voraussetzungen

Sie benoetigen drei Artefakte auf dem Host:

1. **`firecracker`-Binary** — installieren Sie es von [firecracker-microvm/firecracker releases](https://github.com/firecracker-microvm/firecracker/releases). `symbi doctor` meldet, ob es ueber `$PATH` erreichbar ist.
2. **Ein Boot-Kernel-Image (`vmlinux`)** — unkomprimiertes ELF, kein bzImage. Entweder ein vorkompiliertes CI-Artefakt herunterladen (am schnellsten) oder aus dem Upstream-Linux mit der von Firecracker empfohlenen Konfiguration bauen (~10 Minuten auf einem Laptop).
3. **Ein Root-Dateisystem-Image (`rootfs.ext4`)** — eine kleine Linux-Userland (Alpine, BusyBox oder Debian) plus ein init-Skript, das den Symbiont In-VM-Kontrakt umsetzt (siehe unten).

KVM muss ebenfalls auf dem Host verfuegbar sein (`/dev/kvm` lesbar fuer den Benutzer, der `symbi up` ausfuehrt).

---

## Quickstart-Rezept

Dies erzeugt ein funktionsfaehiges ~30 MB grosses `rootfs.ext4` und zieht ein vorkompiliertes `vmlinux` aus den CI-Artefakten von Firecracker. Es ist absichtlich minimal — kein Netzwerk, kein Python, keine zusaetzlichen Laufzeiten. Passen Sie es an Ihren Workload an.

### 1. Vorkompilierten Kernel herunterladen

```bash
ARCH="$(uname -m)"
curl -fsSL "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${ARCH}/vmlinux-5.10.210" \
  -o /var/lib/firecracker/vmlinux
```

Dies ist der Kernel, gegen den die CI von Firecracker testet — bekanntermassen kompatibel mit dem Runtime.

### 2. Minimales Alpine-rootfs bauen

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

### 3. Symbiont In-VM-Init-Kontrakt installieren

Das init-Skript, das als PID 1 innerhalb der microVM laeuft, muss das untenstehende Protokoll umsetzen. Speichern Sie dies als `/mnt/rootfs/sbin/symbi-init`:

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

### 4. Aushaengen und verifizieren

```bash
sudo umount /mnt/rootfs
e2fsck -f /var/lib/firecracker/rootfs.ext4
```

### 5. In ein Symbiont-Projekt einbinden

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/firecracker/vmlinux \
  --firecracker-rootfs /var/lib/firecracker/rootfs.ext4
```

`symbi init` validiert vor dem Scaffolding, dass beide Pfade existieren. Die generierte `symbiont.toml` enthaelt am Ende:

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

Anschliessend fuehrt `symbi up` jeden Agenten in einer eigenen microVM aus.

---

## Der In-VM-Init-Kontrakt

Das ist es, was Ihr rootfs umsetzen muss. Die Runtime-Seite dieses Kontrakts ist fest; die Userspace-Seite gehoert Ihnen.

| Was der Host schreibt | Wohin | Format |
|-----------------------|-------|--------|
| Agent-Code | Host-Arbeitsverzeichnis → in der VM als `/work/code` mounten | ausfuehrbare Datei |
| Umgebungsvariablen | Host-Arbeitsverzeichnis → als `/work/env.json` mounten | JSON-Objekt |

| Was die VM tun muss |
|---------------------|
| `/proc`, `/sys`, `/dev`, `/tmp` mounten |
| `ttyS0` als Konsole hochfahren (Firecrackers serielle Ausgabe → Host-stdout) |
| `/work/code` finden und ausfuehren |
| Exit-Code erfassen, in die Konsole schreiben |
| VM anhalten (`reboot -f` + `panic=1`-Boot-Argument = Shutdown) |

Das Runtime raeumt das Host-Arbeitsverzeichnis auf, nachdem die VM beendet wurde.

### Transport fuer `/work` waehlen

Das obige Beispiel laesst `/work` als Mount-Platzhalter, weil es zwei vertretbare Verdrahtungsmuster gibt und die richtige Wahl von Ihrer Kernel-Konfiguration abhaengt:

- **vsock** — der Host lauscht auf einer vsock-CID, init verbindet sich, empfaengt einen Tarball, extrahiert ihn nach `/work`. Sauberer, kein zweites Laufwerk, aber erfordert `CONFIG_VSOCKETS=y` und einen Userspace-vsock-Client im rootfs.
- **Zweites Block-Device** — uebergeben Sie das Host-Arbeitsverzeichnis als ext4-Image, das als `/dev/vdb` angehaengt wird; init mountet es unter `/work`. Einfachere Kernel-Konfiguration, fuegt aber pro Ausfuehrung einen Image-Schritt auf dem Host hinzu.

Symbionts `FirecrackerRunner` schreibt das Arbeitsverzeichnis derzeit unter `$TMPDIR/symbi-fc-<uuid>/` auf das Host-Dateisystem und erwartet, dass Ihr init weiss, wie es im Inneren der VM zu Tage gefoerdert wird. Waehlen Sie den Transport, der zu Ihrem Kernel passt, und passen Sie `symbi-init` entsprechend an.

---

## Haertungs-Checkliste

Bevor Sie dies fuer Produktionsverkehr einschalten:

- [ ] **Kernel-Konfiguration** — entfernen Sie, was Sie nicht brauchen. Firecracker empfiehlt einen Gast-Kernel, der mit ihrer `microvm-kernel-x86_64.config` gebaut wurde.
- [ ] **rootfs schreibgeschuetzt** — behalten Sie `rootfs_read_only = true` in `[sandbox.firecracker]` bei. Agenten, die Scratch-Speicher benoetigen, schreiben nach `/tmp` (tmpfs).
- [ ] **Kein SSH, kein getty** — der einzige Eintrittspunkt sollte das init-Skript sein.
- [ ] **Privilegien fallen lassen** — fuehren Sie `firecracker` selbst unter einem dedizierten unprivilegierten Benutzer mit `jailer` aus. Das Runtime nimmt dies auf, wenn Sie `firecracker_binary` auf ein Wrapper-Skript setzen.
- [ ] **Speicher-Budget** — setzen Sie `mem_mib` auf den kleinsten Wert, den Ihr Workload toleriert. Niedriger = schnellerer Boot.
- [ ] **Netzwerk** — Agenten, die kein ausgehendes Netzwerk benoetigen, sollten kein TAP-Geraet bekommen. Symbiont legt keines automatisch an.

---

## Fehlerbehebung

**`Firecracker is not available at 'firecracker'`** — installieren Sie das Binary oder setzen Sie `firecracker_binary` in `[sandbox.firecracker]` auf seinen Pfad.

**`Firecracker kernel image not found at ...`** — der Pfad in `kernel_image_path` existiert nicht. `symbi init` validiert dies zur Scaffold-Zeit, sodass dies nach init nicht passieren sollte, ausser Sie haben Dateien verschoben.

**VM bootet, aber Agenten haengen** — Ihr init-Skript findet wahrscheinlich `/work/code` nicht. Verbinden Sie sich mit der seriellen Konsole und pruefen Sie die In-VM-Logs. Das Referenz-init hat eine 1-Sekunden-Warteschleife; verlaengern Sie sie, wenn Ihr Arbeitsverzeichnis-Transport langsam anhaengt.

**`/dev/kvm` permission denied** — der Benutzer, der `symbi up` ausfuehrt, benoetigt Lesezugriff. Fuegen Sie ihn der `kvm`-Gruppe hinzu: `sudo usermod -aG kvm $USER`, dann erneut anmelden.

---

## Was Symbiont nicht ausliefert

Per Design:

- **Kein rootfs-Builder.** Betreibergebiet — die rootfs-Konstruktion variiert zu stark je nach Deployment fuer ein One-size-fits-all-Skript.
- **Kein Kernel-Build-Skript.** Das Firecracker-Projekt veroeffentlicht bereits einen vorkompilierten Kernel und die empfohlene Konfiguration; wir verlinken darauf, anstatt sie zu vendoren.
- **Keine Netzwerk-Verklempnung.** TAP-Setup, NAT, Egress-Richtlinie — alles pro Deployment. Verdrahten Sie es bei Bedarf via `network-interfaces` in der VM-JSON-Konfiguration.
- **Kein Pfad fuer Geheimnisse-ins-rootfs.** Geben Sie Geheimnisse ueber `env.json` weiter (das nur fuer die Dauer der Ausfuehrung im Host-Arbeitsverzeichnis liegt), nicht indem Sie sie ins Image einbacken.

Fuer die meisten Workloads ist Stufe 1 (Docker) oder Stufe 2 (gVisor) die richtige Wahl. Waehlen Sie Stufe 3, wenn Sie speziell eine Kernel-Grenze benoetigen — mandantenfaehiger nicht vertrauenswuerdiger Code, regulierte Daten oder Workloads, bei denen die Granularitaet der Syscall-Filter von gVisor nicht ausreicht.
