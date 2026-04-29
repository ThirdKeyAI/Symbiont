---
layout: default
title: Configuracion de Firecracker (Nivel 3)
nav_order: 8
nav_exclude: true
---

# Configuracion de Firecracker (Nivel 3)

El sandbox de Nivel 3 de Symbiont ejecuta cada agente dentro de una microVM Firecracker dedicada con un kernel Linux real y un sistema de archivos raiz proporcionado por el operador. Este es el nivel de aislamiento mas fuerte que distribuye el runtime — el codigo del agente nunca toca el kernel del host, nunca comparte superficie de kernel con otros inquilinos, y se destruye al final de cada ejecucion.

Esta guia cubre lo que necesita producir antes de apuntar `symbi init --sandbox tier3` a ello.

---

## Por que lo proporciona el operador?

Firecracker es fundamentalmente diferente de Docker o gVisor. Arranca un kernel real contra una imagen de disco que usted proporciona, en lugar de ejecutar un proceso dentro de un namespace de kernel compartido. Esto significa que **usted** controla el contrato de userspace — que esta instalado, como se ejecuta init, como se entrega la carga de trabajo. Symbiont no puede integrar de fabrica un rootfs de talla unica porque las suposiciones de confianza, el presupuesto de rendimiento y las dependencias de runtime varian segun el despliegue.

El lado del runtime esta totalmente implementado: `FirecrackerRunner` escribe una configuracion de VM por ejecucion, deja su codigo en un directorio de trabajo del lado del host, ejecuta `firecracker --no-api --config-file <path>`, y captura la salida desde la consola serie. Lo que usted aporta es el kernel + rootfs que sabe como consumir el directorio de trabajo.

---

## Prerrequisitos

Necesita tres artefactos en el host:

1. **Binario `firecracker`** — instale desde [firecracker-microvm/firecracker releases](https://github.com/firecracker-microvm/firecracker/releases). `symbi doctor` reporta si es alcanzable en `$PATH`.
2. **Una imagen de kernel de arranque (`vmlinux`)** — ELF sin comprimir, no bzImage. Descargue un artefacto preconstruido de CI (lo mas rapido) o compile desde el upstream de Linux con la configuracion recomendada por Firecracker (~10 minutos en una laptop).
3. **Una imagen de sistema de archivos raiz (`rootfs.ext4`)** — un userland Linux pequeno (Alpine, BusyBox o Debian) mas un script de init que implemente el contrato in-VM de Symbiont (vea mas abajo).

KVM tambien debe estar disponible en el host (`/dev/kvm` legible por el usuario que ejecuta `symbi up`).

---

## Receta rapida

Esta produce un `rootfs.ext4` funcional de ~30 MB y descarga un `vmlinux` preconstruido desde los artefactos de CI de Firecracker. Es deliberadamente minima — sin red, sin Python, sin runtimes adicionales. Adaptela para su carga de trabajo.

### 1. Descargue un kernel preconstruido

```bash
ARCH="$(uname -m)"
curl -fsSL "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${ARCH}/vmlinux-5.10.210" \
  -o /var/lib/firecracker/vmlinux
```

Este es el kernel contra el que CI de Firecracker prueba — known-good para el runtime.

### 2. Construya un rootfs Alpine minimo

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

### 3. Instale el contrato de init in-VM de Symbiont

El script de init que se ejecuta como PID 1 dentro de la microVM debe implementar el protocolo de abajo. Guardelo como `/mnt/rootfs/sbin/symbi-init`:

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

### 4. Desmonte y verifique

```bash
sudo umount /mnt/rootfs
e2fsck -f /var/lib/firecracker/rootfs.ext4
```

### 5. Cablee todo a un proyecto Symbiont

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/firecracker/vmlinux \
  --firecracker-rootfs /var/lib/firecracker/rootfs.ext4
```

`symbi init` valida que ambas rutas existan antes de generar el esqueleto. El `symbiont.toml` generado termina con:

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

Luego `symbi up` ejecuta cada agente dentro de su propia microVM.

---

## El contrato de init in-VM

Esto es lo que su rootfs debe implementar. El lado del runtime de este contrato es fijo; el lado del userspace es suyo.

| Lo que escribe el host | Donde | Formato |
|------------------------|-------|---------|
| Codigo del agente | dir. de trabajo del host → montar como `/work/code` dentro de la VM | archivo ejecutable |
| Variables de entorno | dir. de trabajo del host → montar como `/work/env.json` | objeto JSON |

| Lo que la VM debe hacer |
|-------------------------|
| Montar `/proc`, `/sys`, `/dev`, `/tmp` |
| Levantar `ttyS0` como la consola (salida serie de Firecracker → stdout del host) |
| Localizar `/work/code` y ejecutarlo |
| Capturar el codigo de salida, escribirlo a la consola |
| Detener la VM (`reboot -f` + arg de arranque `panic=1` = apagado) |

El runtime limpia el directorio de trabajo del host despues de que la VM sale.

### Eligiendo un transporte para `/work`

El ejemplo de arriba deja `/work` como un montaje de marcador de posicion porque hay dos patrones de cableado razonables y la eleccion correcta depende de su configuracion de kernel:

- **vsock** — el host escucha en un CID de vsock, init se conecta, recibe un tarball, lo extrae a `/work`. Mas limpio, sin segunda unidad, pero requiere `CONFIG_VSOCKETS=y` y un cliente vsock de userspace en el rootfs.
- **Segundo dispositivo de bloque** — pase el dir. de trabajo del host como una imagen ext4 adjunta como `/dev/vdb`, init la monta en `/work`. Configuracion de kernel mas simple, pero anade un paso de imagen por ejecucion en el host.

`FirecrackerRunner` de Symbiont actualmente escribe el dir. de trabajo en el sistema de archivos del host bajo `$TMPDIR/symbi-fc-<uuid>/` y espera que su init sepa como exponerlo dentro de la VM. Elija el transporte que se adapte a su kernel y actualice `symbi-init` en consecuencia.

---

## Lista de verificacion de endurecimiento

Antes de activar esto para trafico de produccion:

- [ ] **Configuracion del kernel** — quite lo que no necesite. Firecracker recomienda un kernel invitado construido con su `microvm-kernel-x86_64.config`.
- [ ] **Rootfs solo lectura** — mantenga `rootfs_read_only = true` en `[sandbox.firecracker]`. Los agentes que necesiten espacio de scratch escriben en `/tmp` (tmpfs).
- [ ] **Sin SSH, sin getty** — el unico punto de entrada debe ser el script de init.
- [ ] **Reduzca privilegios** — ejecute el propio `firecracker` bajo un usuario sin privilegios dedicado con `jailer`. El runtime lo recogera si configura `firecracker_binary` apuntando a un script wrapper.
- [ ] **Presupuesto de memoria** — fije `mem_mib` al valor mas pequeno que tolere su carga de trabajo. Menor = arranque mas rapido.
- [ ] **Red** — los agentes que no necesiten red de salida no deberian recibir un dispositivo TAP. Symbiont no crea uno automaticamente.

---

## Resolucion de problemas

**`Firecracker is not available at 'firecracker'`** — instale el binario o configure `firecracker_binary` en `[sandbox.firecracker]` con su ruta.

**`Firecracker kernel image not found at ...`** — la ruta en `kernel_image_path` no existe. `symbi init` valida esto en tiempo de scaffolding, asi que esto no deberia pasar despues de init a menos que haya movido archivos.

**La VM arranca pero los agentes se cuelgan** — su script de init probablemente no encuentra `/work/code`. Conectese a la consola serie y revise los logs in-VM. El init de referencia tiene un bucle de espera de 1 segundo; alarguelo si el transporte de su dir. de trabajo tarda en adjuntarse.

**`/dev/kvm` permiso denegado** — el usuario que ejecuta `symbi up` necesita acceso de lectura. Agreguelo al grupo `kvm`: `sudo usermod -aG kvm $USER`, luego vuelva a iniciar sesion.

---

## Lo que Symbiont no distribuye

Por diseno:

- **Sin constructor de rootfs.** Territorio del operador — la construccion del rootfs varia demasiado segun el despliegue para un script de talla unica.
- **Sin script de compilacion del kernel.** El proyecto Firecracker ya publica un kernel preconstruido y la configuracion recomendada; enlazamos a ellos en lugar de empaquetarlos.
- **Sin plomeria de red.** Configuracion de TAP, NAT, politica de egreso — todo por despliegue. Cablee via `network-interfaces` en la configuracion JSON de la VM si lo necesita.
- **Sin ruta para meter secretos en el rootfs.** Pase los secretos via `env.json` (que vive solo en el dir. de trabajo del host por la duracion de la ejecucion), no integrandolos en la imagen.

Para la mayoria de las cargas de trabajo, el Nivel 1 (Docker) o el Nivel 2 (gVisor) es la opcion adecuada. Elija el Nivel 3 cuando necesite especificamente una frontera de kernel — codigo no confiable multi-inquilino, datos regulados o cargas de trabajo donde la granularidad del filtro de syscalls de gVisor no es suficiente.
