# Acceso saliente opcional

Esta función introducida en 3.0 se activa de forma explícita. Un perfil
`[outbound]` controlado por el operador limita cada herramienta HTTP gobernada de
ToolClad a una operación con origen, método HTTP, límite de ruta y cuota de
solicitudes definidos. Sin ese perfil se mantiene el comportamiento anterior.

Las credenciales proceden únicamente de la variable de entorno o del almacén de
secretos seleccionado. Un fallo del almacén nunca activa una fuente alternativa.
El runtime incorpora el secreto al enviar la solicitud y bloquea respuestas que
contengan su valor exacto. Esto no constituye una protección general contra la
exfiltración de datos.

Cada ejecución recibe una autorización revocable propia. Cancelarla impide nuevas
admisiones, pero una solicitud ya recibida puede producir efectos en el servicio.
Los resultados inciertos requieren inspección y conciliación, no repetición
automática. Las escrituras pueden exigir la aprobación exacta por terminal existente.

Los parámetros de ruta se rechazan por defecto. `allow_path_parameters = true`
permite parámetros normales, pero nunca segmentos como `..%3B`.
`allowed_query_keys` limita opcionalmente las claves de consulta decodificadas y
rechaza claves repetidas. Las llamadas concurrentes comparten una única resolución
exitosa de credenciales por ejecución; la revocación vacía esa caché y los fallos
pueden reintentarse. Los hashes del diario excluyen los tokens de Vault y los
Secret ID de AppRole. Los secretos reflejados en nombres o valores de cabeceras
se rechazan antes de calcular sus hashes. La `version` declarada identifica una
rotación intencionada.

Vault se autentica con el método configurado antes de leer. Las variables de entorno de tokens o TLS de Vault no sustituyen esa configuración. La detección de reflejos contempla los nombres de cabecera convertidos a minúsculas y los espacios que HTTP elimina en los extremos de sus valores.

`symbi tools validate` y `symbi doctor` comprueban la configuración sin recuperar
la credencial de la aplicación. Los controles son OSS y conservan el uso de
Landlock en Linux sin exigir otro proxy, contenedor o VM. Otros transportes y la
ejecución de navegador quedan fuera de esta función. Las pruebas CLI conservadas
no certifican el aislamiento de un sandbox.

La [configuración y migración completas en inglés](/outbound-capabilities) incluyen
ejemplos TOML, límites y cambios para usuarios de Rust. El
[catálogo de controles](/security/README) separa implementación y validación.

Migración de la API Rust: añada `HttpDef.capability: None`, `InferenceBrokerConfig.credential: None` y `ToolEffect::NetworkRequestStarted.outbound: None` a los literales de estructuras. `SecurityTier` ya no implementa un orden total (`Ord`). Use `satisfies` para requisitos mínimos; Landlock no satisface requisitos de contenedores o máquinas virtuales, ni a la inversa.
