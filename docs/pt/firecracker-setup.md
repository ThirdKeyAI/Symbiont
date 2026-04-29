---
layout: default
title: Configuração do Firecracker (Tier 3)
nav_order: 8
nav_exclude: true
---

# Configuração do Firecracker (Tier 3)

O sandbox Tier 3 do Symbiont executa cada agente dentro de uma microVM Firecracker dedicada, com um kernel Linux real e um sistema de arquivos raiz fornecido pelo operador. Esta é a camada de isolamento mais forte que o runtime entrega — o código do agente nunca toca o kernel do host, nunca compartilha uma superfície de kernel com outros tenants, e é destruído ao final de cada execução.

Este guia cobre o que você precisa produzir antes de apontar `symbi init --sandbox tier3` para ele.

---

## Por que fornecido pelo operador?

O Firecracker é fundamentalmente diferente do Docker ou gVisor. Ele inicializa um kernel real contra uma imagem de disco que você fornece, em vez de executar um processo dentro de um namespace de kernel compartilhado. Isso significa que **você** controla o contrato do espaço de usuário — o que está instalado, como o init roda, como a carga de trabalho é entregue. O Symbiont não pode embutir um rootfs único para todos porque as suposições de confiança, o orçamento de performance e as dependências de runtime variam por implantação.

O lado do runtime está totalmente implementado: `FirecrackerRunner` escreve uma configuração de VM por execução, coloca seu código em um diretório de trabalho do lado do host, executa `firecracker --no-api --config-file <path>`, e captura a saída do console serial. O que você traz é o kernel + rootfs que sabe como consumir o diretório de trabalho.

---

## Pré-requisitos

Você precisa de três artefatos no host:

1. **Binário `firecracker`** — instale a partir das [releases do firecracker-microvm/firecracker](https://github.com/firecracker-microvm/firecracker/releases). `symbi doctor` reporta se está acessível em `$PATH`.
2. **Uma imagem de kernel de boot (`vmlinux`)** — ELF não comprimido, não bzImage. Baixe um artefato de CI pré-construído (mais rápido) ou compile a partir do Linux upstream com a configuração recomendada do Firecracker (~10 minutos em um laptop).
3. **Uma imagem de sistema de arquivos raiz (`rootfs.ext4`)** — um pequeno espaço de usuário Linux (Alpine, BusyBox ou Debian) mais um script de init que implementa o contrato do Symbiont dentro da VM (veja abaixo).

KVM também deve estar disponível no host (`/dev/kvm` legível pelo usuário que executa `symbi up`).

---

## Receita de início rápido

Isso produz um `rootfs.ext4` funcional de ~30 MB e baixa um `vmlinux` pré-construído dos artefatos de CI do Firecracker. É intencionalmente mínimo — sem rede, sem Python, sem runtimes extras. Adapte para sua carga de trabalho.

### 1. Baixar um kernel pré-construído

```bash
ARCH="$(uname -m)"
curl -fsSL "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${ARCH}/vmlinux-5.10.210" \
  -o /var/lib/firecracker/vmlinux
```

Este é o kernel contra o qual o CI do Firecracker testa — sabidamente bom para o runtime.

### 2. Construir um rootfs Alpine mínimo

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

### 3. Instalar o contrato de init dentro da VM do Symbiont

O script de init que executa como PID 1 dentro da microVM deve implementar o protocolo abaixo. Salve isto como `/mnt/rootfs/sbin/symbi-init`:

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

### 4. Desmontar e verificar

```bash
sudo umount /mnt/rootfs
e2fsck -f /var/lib/firecracker/rootfs.ext4
```

### 5. Conectar a um projeto Symbiont

```bash
symbi init --profile assistant --sandbox tier3 \
  --firecracker-kernel /var/lib/firecracker/vmlinux \
  --firecracker-rootfs /var/lib/firecracker/rootfs.ext4
```

`symbi init` valida que ambos os caminhos existem antes do scaffold. O `symbiont.toml` gerado fica com:

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

Em seguida, `symbi up` executa cada agente dentro de sua própria microVM.

---

## O contrato de init dentro da VM

Isto é o que seu rootfs deve implementar. O lado do runtime deste contrato é fixo; o lado do espaço de usuário é seu.

| O que o host escreve | Onde | Formato |
|----------------------|------|---------|
| Código do agente | dir de trabalho do host → monte como `/work/code` dentro da VM | arquivo executável |
| Variáveis de ambiente | dir de trabalho do host → monte como `/work/env.json` | objeto JSON |

| O que a VM deve fazer |
|-----------------------|
| Montar `/proc`, `/sys`, `/dev`, `/tmp` |
| Subir `ttyS0` como console (saída serial do Firecracker → stdout do host) |
| Localizar `/work/code` e executá-lo |
| Capturar código de saída, escrever no console |
| Parar a VM (`reboot -f` + arg de boot `panic=1` = shutdown) |

O runtime limpa o dir de trabalho do host depois que a VM sai.

### Escolhendo um transporte para `/work`

O exemplo acima deixa `/work` como uma montagem placeholder porque há dois padrões de fiação razoáveis e a escolha certa depende da configuração do seu kernel:

- **vsock** — o host escuta em um CID vsock, o init conecta, recebe um tarball, extrai para `/work`. Mais limpo, sem segundo drive, mas requer `CONFIG_VSOCKETS=y` e um cliente vsock no espaço de usuário do rootfs.
- **Segundo dispositivo de bloco** — passar o dir de trabalho do host como uma imagem ext4 anexada como `/dev/vdb`, o init monta em `/work`. Configuração de kernel mais simples, mas adiciona uma etapa de imagem por execução no host.

O `FirecrackerRunner` do Symbiont atualmente escreve o dir de trabalho no sistema de arquivos do host em `$TMPDIR/symbi-fc-<uuid>/` e espera que seu init saiba como expô-lo dentro da VM. Escolha o transporte que se adequa ao seu kernel e atualize `symbi-init` em conformidade.

---

## Checklist de hardening

Antes de ativar isto para tráfego de produção:

- [ ] **Configuração do kernel** — remova o que você não precisa. O Firecracker recomenda um kernel guest construído com o `microvm-kernel-x86_64.config` deles.
- [ ] **Rootfs somente leitura** — mantenha `rootfs_read_only = true` em `[sandbox.firecracker]`. Agentes que precisam de espaço de scratch escrevem em `/tmp` (tmpfs).
- [ ] **Sem SSH, sem getty** — o único ponto de entrada deve ser o script de init.
- [ ] **Reduzir privilégios** — execute o próprio `firecracker` sob um usuário dedicado não privilegiado com `jailer`. O runtime captura isso se você definir `firecracker_binary` para um script wrapper.
- [ ] **Orçamento de memória** — defina `mem_mib` para o menor valor que sua carga de trabalho tolera. Menor = boot mais rápido.
- [ ] **Rede** — agentes que não precisam de rede de saída não devem receber um dispositivo TAP. O Symbiont não cria um automaticamente.

---

## Solução de problemas

**`Firecracker is not available at 'firecracker'`** — instale o binário ou defina `firecracker_binary` em `[sandbox.firecracker]` para o seu caminho.

**`Firecracker kernel image not found at ...`** — o caminho em `kernel_image_path` não existe. `symbi init` valida isso no momento do scaffold, então isso não deve acontecer pós-init a menos que você tenha movido arquivos.

**A VM dá boot mas os agentes travam** — seu script de init provavelmente não está encontrando `/work/code`. Conecte-se ao console serial e verifique os logs dentro da VM. O init de referência tem um loop de espera de 1 segundo; aumente-o se o transporte do dir de trabalho for lento para anexar.

**`/dev/kvm` permission denied** — o usuário que executa `symbi up` precisa de acesso de leitura. Adicione-o ao grupo `kvm`: `sudo usermod -aG kvm $USER`, depois faça login novamente.

---

## O que o Symbiont não entrega

Por design:

- **Sem construtor de rootfs.** Território do operador — a construção do rootfs varia demais por implantação para um script único para todos.
- **Sem script de build de kernel.** O projeto Firecracker já publica um kernel pré-construído e a configuração recomendada; nós linkamos para esses em vez de vendorizar.
- **Sem encanamento de rede.** Configuração de TAP, NAT, política de saída — tudo por implantação. Conecte via `network-interfaces` na configuração JSON da VM se você precisar.
- **Sem caminho para colocar segredos no rootfs.** Passe segredos via `env.json` (que vive apenas no dir de trabalho do host pela duração da execução), não embutindo-os na imagem.

Para a maioria das cargas de trabalho, Tier 1 (Docker) ou Tier 2 (gVisor) é a escolha certa. Escolha Tier 3 quando você especificamente precisar de um limite de kernel — código não confiável multi-tenant, dados regulamentados, ou cargas de trabalho onde a granularidade do filtro de syscalls do gVisor não é suficiente.
