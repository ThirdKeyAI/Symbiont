# Symbi Shell — Orquestacion Interactiva de Agentes

> **Status: Beta.** `symbi shell` es utilizable en el dia a dia pero la superficie de comandos, los atajos de teclado y los formatos de persistencia aun pueden cambiar entre versiones menores. Presente issues en [thirdkeyai/symbiont](https://github.com/thirdkeyai/symbiont/issues) con la etiqueta `shell`.

`symbi shell` es una interfaz de usuario de terminal basada en [ratatui](https://ratatui.rs) para construir, orquestar y operar agentes de Symbiont. Se apoya sobre el mismo runtime que `symbi up` y `symbi run`, pero lo expone como una sesion interactiva con autoria conversacional, orquestacion en vivo y conexion remota.

## Cuando usar la shell

| Caso de uso | Comando |
|-------------|---------|
| Preparar un proyecto e iterar sobre agentes, herramientas y politicas con asistencia de LLM | `symbi shell` |
| Ejecutar un agente hasta completarse sin un bucle interactivo | `symbi run <agent> -i <json>` |
| Iniciar el runtime completo para webhooks, cron y adaptadores de chat | `symbi up` |

La shell es el punto de entrada predeterminado para autoria. Los comandos no interactivos son mejores dentro de CI, trabajos cron y pipelines de despliegue.

## Ejecucion

```bash
symbi shell                    # iniciar una sesion nueva
symbi shell --list-sessions    # mostrar sesiones guardadas y salir
symbi shell --resume <id>      # reabrir una sesion por UUID
```

`--resume` acepta tanto un UUID como el nombre de un snapshot guardado previamente con `/snapshot`.

## Disposicion

La shell usa un viewport en linea que comparte la terminal con tu scrollback existente. Veras, de arriba hacia abajo:

- **Barra lateral de estructura del proyecto** (conmutable) — arbol de archivos del proyecto actual, resaltando agentes, politicas y herramientas.
- **Linea de tiempo de traza** — tarjetas coloreadas por fase de ORGA para Observe, Reason, Gate y Act, transmitiendose en tiempo real durante llamadas al LLM.
- **Tarjeta de agente** — metadatos, politicas e invocaciones recientes del agente actualmente seleccionado.
- **Linea de entrada** — escribe `/command` o prosa libre. `@mention` inserta rutas y agentes mediante autocompletado difuso.

El resaltado de sintaxis cubre el DSL de Symbiont, Cedar y los manifiestos de ToolClad a traves de gramaticas tree-sitter.

### Atajos de teclado

| Atajo | Accion |
|-------|--------|
| `Enter` | Enviar entrada (funciona incluso cuando el popup de autocompletado esta visible) |
| `/` o `@` | Abrir automaticamente el popup de autocompletado |
| `↑` / `↓` | Navegar el historial de entrada o las entradas del popup |
| `Ctrl+R` | Busqueda inversa en el historial |
| `Tab` | Aceptar el autocompletado resaltado |
| `Esc` | Cerrar el popup / cancelar una llamada al LLM en curso |
| `Ctrl+L` | Limpiar el buffer visible de salida |
| `Ctrl+D` | Salir de la shell |

Bajo Zellij, la shell detecta el multiplexor e imprime una advertencia de compatibilidad del viewport en linea; usa `--full-screen` si prefieres ejecutar en un buffer de pantalla alternativa.

## Catalogo de comandos

Los comandos estan agrupados por proposito. Cada comando acepta `help` / `--help` / `-h` para imprimir una breve descripcion de uso sin despachar al orquestador.

### Autoria

| Comando | Que hace |
|---------|----------|
| `/init [profile\|description]` | Prepara un proyecto de Symbiont. Los nombres de perfil conocidos (`minimal`, `assistant`, `dev-agent`, `multi-agent`) ejecutan un scaffold determinista; cualquier otro texto se trata como una descripcion libre que el orquestador usa para elegir un perfil. |
| `/spawn <description>` | Genera un agente en DSL a partir de prosa. El resultado se valida contra las restricciones del proyecto antes de escribirse en `agents/`. |
| `/policy <requirement>` | Genera una politica Cedar para el requisito descrito y la valida. |
| `/tool <description>` | Genera un manifiesto `.clad.toml` de ToolClad y lo valida. |
| `/behavior <description>` | Genera un bloque de behavior reutilizable en DSL y lo valida. |

Los comandos de autoria escriben a disco solo despues de que pasa la validacion. Las violaciones de restricciones se explican en la linea de tiempo de traza con errores a nivel de linea precisa.

### Orquestacion

| Comando | Patron |
|---------|--------|
| `/run <agent> [input]` | Iniciar o re-ejecutar un agente. |
| `/ask <agent> <message>` | Enviar un mensaje a un agente y esperar la respuesta. |
| `/send <agent> <message>` | Enviar un mensaje sin esperar la respuesta. |
| `/chain <a,b,c> <input>` | Canaliza la salida de cada agente hacia el siguiente. |
| `/parallel <a,b,c> <input>` | Ejecutar agentes en paralelo con la misma entrada; agregar resultados. |
| `/race <a,b,c> <input>` | Ejecutar en paralelo; la primera respuesta exitosa gana y el resto se cancelan. |
| `/debate <a,b,c> <topic>` | Debate multi-agente estructurado sobre un tema. |
| `/exec <command>` | Ejecutar un comando shell dentro del agente de desarrollo aislado en sandbox. |

### Operaciones

| Comando | Que hace |
|---------|----------|
| `/agents` | Lista los agentes activos. |
| `/monitor [agent]` | Transmite el estado en vivo del agente indicado (o de todos). |
| `/logs [agent]` | Muestra los logs recientes. |
| `/audit [filter]` | Muestra las entradas recientes del rastro de auditoria; filtra por agente, decision o rango de tiempo. |
| `/doctor` | Diagnostica el entorno del runtime local. |
| `/memory <agent> [query]` | Consulta la memoria de un agente. |
| `/debug <agent>` | Inspecciona el estado interno de un agente. |
| `/pause`, `/resume-agent`, `/stop`, `/destroy` | Controles del ciclo de vida del agente. |

### Herramientas, skills y verificacion

| Comando | Que hace |
|---------|----------|
| `/tools [list\|add\|remove]` | Gestiona las herramientas ToolClad disponibles para los agentes. |
| `/skills [list\|install\|remove]` | Gestiona los skills disponibles para los agentes. |
| `/verify <artifact>` | Verifica un artefacto firmado (manifiesto de herramienta, skill) contra su firma SchemaPin. |

### Programacion

| Comando | Que hace |
|---------|----------|
| `/cron list` | Lista los trabajos de agente programados. |
| `/cron add` / `/cron remove` | Crea o elimina trabajos programados. |
| `/cron history` | Muestra ejecuciones recientes. |

`/cron` funciona tanto localmente como sobre una conexion remota (ver abajo). Consulta la [guia de Programacion](/scheduling) para el motor cron completo.

### Canales

| Comando | Que hace |
|---------|----------|
| `/channels` | Lista los adaptadores de canal registrados (Slack, Teams, Mattermost). |
| `/connect <channel>` | Registra un nuevo adaptador de canal. |
| `/disconnect <channel>` | Elimina un adaptador. |

La gestion de canales requiere una conexion remota cuando apunta a un runtime desplegado.

### Secretos

| Comando | Que hace |
|---------|----------|
| `/secrets list\|set\|get\|remove` | Gestiona secretos en el almacen local cifrado del runtime. |

Los secretos se cifran en reposo con `SYMBIONT_MASTER_KEY` y estan delimitados por agente.

### Despliegue (Beta)

> **Status: Beta.** El stack de despliegue es de un solo agente en la edicion OSS. Los despliegues multi-agente y gestionados estan en el roadmap.

| Comando | Destino |
|---------|---------|
| `/deploy local` | Docker con un runner de sandbox endurecido en el demonio Docker local. |
| `/deploy cloudrun` | Google Cloud Run — construye una imagen, la sube y despliega un servicio. |
| `/deploy aws` | AWS App Runner. |

`/deploy` lee el agente activo y la configuracion del proyecto y produce un artefacto de despliegue reproducible. Para topologias multi-agente, despliega el coordinador y cada worker por separado y conectalos con mensajeria entre instancias (ver [Arquitectura del Runtime](/runtime-architecture#cross-instance-agent-messaging)).

### Conexion remota

| Comando | Que hace |
|---------|----------|
| `/attach <url>` | Conecta esta shell a un runtime remoto sobre HTTP o HTTPS. |
| `/detach` | Se desconecta del runtime actualmente conectado. |

Usa `https://` para cualquier destino remoto o de produccion — el canal de attach transporta tokens de autenticacion y trafico de operaciones, por lo que HTTP en texto plano solo es apropiado para desarrollo local. El atajo `local` usa por defecto `http://localhost:8080`, y las URLs proporcionadas sin un esquema explicito se prefijan con `http://` para preservar la ergonomia del desarrollo en loopback; para todo lo demas, pasa una URL completa `https://...`.

Una vez conectada, `/cron`, `/channels`, `/agents`, `/audit` y la mayoria de los comandos de operaciones actuan sobre el runtime remoto en lugar del local. `/secrets` permanece local — los secretos remotos se quedan en el almacen del runtime remoto.

### Gestion de sesiones

| Comando | Que hace |
|---------|----------|
| `/snapshot [name]` | Guarda la sesion actual. |
| `/resume <snapshot>` | Restaura un snapshot guardado. |
| `/export <path>` | Exporta el transcript de la conversacion a disco. |
| `/new` | Inicia una sesion nueva, descartando la actual. |
| `/compact [limit]` | Compacta el historial de la conversacion para ajustarse a un presupuesto de tokens. |
| `/context` | Muestra la ventana de contexto actual y el uso de tokens. |

Las sesiones se almacenan bajo `.symbi/sessions/<uuid>/`. La shell auto-activa la compactacion cuando el contexto crece mas alla del presupuesto configurado.

### Controles de sesion

| Comando | Que hace |
|---------|----------|
| `/model [name]` | Muestra o cambia el modelo de inferencia activo. |
| `/cost` | Muestra los totales de tokens y coste de API para la sesion. |
| `/status` | Muestra el estado del runtime y de la sesion. |
| `/dsl` | Alterna entre modos de entrada DSL y orquestador — el modo DSL evalua en el proceso. |
| `/clear` | Limpia el buffer visible de salida (el historial se preserva). |
| `/quit` / `/exit` | Salir de la shell. |
| `/help` | Muestra el catalogo de comandos. |

## Modo DSL

Presiona `/dsl` para cambiar la linea de entrada al modo DSL. En modo DSL, la shell analiza y evalua la entrada contra el interprete DSL en proceso con autocompletado y errores respaldados por tree-sitter, sin enrutar a traves del orquestador. Alterna de vuelta con `/dsl` otra vez.

## Restricciones y validacion

Los comandos de autoria aplican un pipeline de validacion local:

1. Los artefactos generados se parsean contra la gramatica del DSL de Symbiont, Cedar o ToolClad segun corresponda.
2. Un cargador de restricciones verifica el resultado contra las restricciones a nivel de proyecto (por ejemplo, capacidades prohibidas, politicas requeridas).
3. Solo despues de que ambos pasos tienen exito se escribe el artefacto a disco.

El LLM orquestador puede ver los efectos del archivo de restricciones a traves de errores de validacion, pero no puede modificar el archivo en si — este es el mismo modelo de confianza usado por el pipeline `symbi tools validate`.

## Advertencias de Beta

Las siguientes partes de la shell siguen bajo desarrollo activo y pueden cambiar sin una ventana de deprecacion:

- `/branch` y `/copy` (ramificacion de sesiones) son comandos reservados y actualmente imprimen un stub de "planeado para una version futura".
- `/deploy cloudrun` y `/deploy aws` son solo de un agente.
- El formato de snapshot y la disposicion de `.symbi/sessions/` pueden cambiar entre versiones menores; usa `/export` si necesitas transcripts duraderos.
- Las heuristicas de autocompletado difuso y la disposicion de la linea de tiempo de traza se ajustan en base a retroalimentacion y pueden cambiar.

Si necesitas una superficie estable hoy, prefiere `symbi up`, `symbi run` y la [API HTTP](/api-reference) — esas estan cubiertas por las garantias de compatibilidad en `SECURITY.md`.

## Ver tambien

- [Primeros Pasos](/getting-started) — instalacion y `symbi init`
- [Guia DSL](/dsl-guide) — referencia del lenguaje de definicion de agentes
- [ToolClad](/toolclad) — contratos declarativos de herramientas
- [Programacion](/scheduling) — motor cron y enrutamiento de entregas
- [Modelo de Seguridad](/security-model) — limites de confianza y aplicacion de politicas
