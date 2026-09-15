---
layout: default
title: Configuração do Firecracker (Tier 3)
nav_order: 8
nav_exclude: true
---

# Configuração do Firecracker (Tier 3)

Comandos de execução única e parsers personalizados usam uma microVM nova com
transporte vsock. A imagem deve conter a versão correspondente de
`symbi-sandbox-guest` como PID 1. O supervisor mantém o controle do processo da VM,
seu prazo e sua remoção. O resultado inclui o identificador da chamada, código de
saída e limites de saída; inicializar ou encerrar a VM não comprova sucesso.

O Tier 3 faz parte do runtime de código aberto. Não exige chave de licença nem
compilação Enterprise: os crates `symbi-sandbox-guest` e `symbi-sandbox-supervisor`
são distribuídos neste repositório, de modo que você pode compilar, auditar e
reproduzir a imagem do convidado por conta própria.

A receita antiga baseada em `/work/code` e console serial foi removida: ela não
implementava o transporte necessário. Consulte o
[guia atualizado em inglês](../firecracker-setup.md) para compilar o serviço,
preparar uma imagem de teste, configurar o projeto e executar os testes reais.

O transporte MCP stdio usa a mesma microVM durante descoberta, verificação de
assinatura e chamada da ferramenta. Sessões PTY usam um terminal de controle
dentro da VM com o protocolo versão 3; imagens antigas precisam ser reconstruídas.
Os adaptadores para CLI
gerenciada e browser ainda não estão disponíveis. A configuração do host, o kernel, a imagem e o VMM
continuam sendo componentes confiáveis. Os testes não constituem uma garantia de
contenção completa.
