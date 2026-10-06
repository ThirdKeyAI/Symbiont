---
layout: default
title: Configuração do Firecracker (Tier 3)
nav_order: 8
nav_exclude: true
---

# Configuração do Firecracker (Tier 3)

O Tier 3 executa comandos de execução única, parsers de saída personalizados,
servidores MCP stdio, sessões PTY e workers de CLI gerenciada em uma microVM
Firecracker nova. O limite ToolClad selecionado e o `FirecrackerRunner` público
usam o mesmo protocolo de convidado e o mesmo supervisor independente. A imagem
deve conter a versão correspondente de `symbi-sandbox-guest` como PID 1. O
supervisor mantém o controle do processo da VM, seu prazo e sua remoção. A
inicialização da VM ou a saída do VMM não substituem o resultado do comando
solicitado.

O Tier 3 faz parte do runtime de código aberto. Não exige chave de licença nem
compilação Enterprise: os crates `symbi-sandbox-guest` e `symbi-sandbox-supervisor`
são distribuídos neste repositório, de modo que você pode compilar, auditar e
reproduzir a imagem do convidado por conta própria.

As operações fixas `read_file`, `list_files` e `grep_files` usam `source_roots`
somente leitura explícitas através do broker de arquivos delimitado do runtime;
essas raízes nunca viram montagens do convidado. Os arquivos declarados de
comando/MCP/PTY usam transferências de bytes delimitadas do protocolo 5 e um teto
separado de `output_roots` para novas saídas no host. O Git usa um fluxo de
snapshot delimitado à parte, com um sistema de arquivos de convidado selado.

A receita antiga baseada em `/work/code` e console serial foi removida: ela não
implementava o transporte necessário. Consulte o
[guia atualizado em inglês](/firecracker-setup) para compilar o serviço, preparar
uma imagem de teste, configurar o projeto, entender os limites de tempo de vida e
executar os testes reais.

O transporte MCP stdio usa a mesma microVM durante descoberta, verificação de
assinatura e chamada da ferramenta. Sessões PTY usam um terminal de controle
dentro da VM. O protocolo atual é a versão 5; imagens construídas para as versões
1 a 4 são deliberadamente recusadas e precisam ser reconstruídas. Os workers de
CLI gerenciada já usam este caminho e exigem Python 3 e a CLI nativa na rootfs
somente leitura; o runtime emite apenas duas capacidades do convidado para o host
(o broker de ferramentas governado e o broker de inferência protegido). A execução
isolada de navegador continua indisponível, e os caminhos selecionados que não são
suportados falham explicitamente.

O serviço de host gerenciado opcional provisiona artefatos aprovados, o jailer,
cgroups de host e admissão com sobrecarga; ele ainda não foi validado para
implantação. A admissão de workers do supervisor comum por usuário compartilha as
reservas configuradas de CPU e memória do convidado com os workers Docker/gVisor
que usam o mesmo diretório de estado. A configuração do host, o kernel, a imagem,
o VMM, a conta de serviço e o armazenamento do host continuam sendo componentes
confiáveis. Os testes não constituem uma garantia de contenção completa.
