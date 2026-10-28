# Acesso de saída opcional

Este recurso introduzido na versão 3.0 depende de ativação explícita. Um perfil
`[outbound]` controlado pelo operador limita cada ferramenta HTTP governada do
ToolClad a uma operação com origem, método HTTP, limite de caminho e cota de
requisições definidos. Sem o perfil, o comportamento anterior é mantido.

As credenciais vêm exclusivamente da variável de ambiente ou do armazenamento de
segredos selecionado. Uma falha no armazenamento nunca ativa uma fonte alternativa.
O runtime insere o segredo no envio e bloqueia respostas que contenham seu valor
exato. Isso não representa proteção geral contra vazamento de dados.

Cada execução recebe sua própria autorização revogável. O cancelamento impede
novas admissões, mas uma requisição já recebida ainda pode produzir efeitos no
serviço. Resultados incertos exigem inspeção e reconciliação, sem repetição
automática. Escritas podem exigir a aprovação exata pelo terminal já existente.

Parâmetros de caminho são recusados por padrão. `allow_path_parameters = true`
permite parâmetros comuns, mas nunca segmentos como `..%3B`.
`allowed_query_keys` restringe opcionalmente as chaves de consulta decodificadas e
recusa chaves repetidas. Chamadas simultâneas compartilham uma única resolução
bem-sucedida de credenciais por execução; a revogação limpa esse cache e falhas
podem ser tentadas novamente. Os hashes do diário excluem tokens do Vault e
Secret IDs do AppRole. Segredos refletidos em nomes ou valores de cabeçalhos são
recusados antes do cálculo de hashes. A `version` declarada identifica uma rotação
intencional.

O Vault autentica com o método configurado antes da leitura. Variáveis de ambiente de tokens ou TLS do Vault não substituem essa configuração. A detecção de reflexos considera nomes de cabeçalhos convertidos para minúsculas e espaços que o HTTP remove das extremidades dos valores.

`symbi tools validate` e `symbi doctor` verificam a configuração sem buscar a
credencial da aplicação. Os controles permanecem OSS e preservam o uso de Landlock
no Linux sem exigir outro proxy, contêiner ou VM. Outros transportes e a execução
de navegador ficam fora deste recurso. Os testes CLI preservados não qualificam
o isolamento de um sandbox.

A [configuração e migração completas em inglês](/outbound-capabilities) incluem
exemplos TOML, limites e alterações para usuários de Rust. O
[catálogo de controles](/security/README) separa implementação e validação.

Migração da API Rust: acrescente `HttpDef.capability: None`, `InferenceBrokerConfig.credential: None` e `ToolEffect::NetworkRequestStarted.outbound: None` aos literais de estruturas. `SecurityTier` deixa de implementar uma ordem total (`Ord`). Use `satisfies` para requisitos mínimos; Landlock não satisfaz requisitos de contêineres ou máquinas virtuais, nem o inverso.
