---
layout: default
title: Configuracion de Firecracker (Nivel 3)
nav_order: 8
nav_exclude: true
---

# Configuracion de Firecracker (Nivel 3)

El Nivel 3 ejecuta comandos oneshot, parsers de salida personalizados, servidores
MCP stdio, sesiones PTY y workers de CLI administrada en una microVM Firecracker
nueva. El limite ToolClad seleccionado y el `FirecrackerRunner` publico usan el
mismo protocolo de invitado y el mismo supervisor independiente. El arranque de la
VM o la salida del VMM no pueden sustituir al resultado del comando solicitado. El
kernel del host, el VMM, la imagen y la configuracion del operador siguen siendo
componentes de confianza. El aislamiento no garantiza que todo escape sea imposible.

El Nivel 3 forma parte del runtime de codigo abierto. No requiere clave de
licencia ni compilacion Enterprise: los crates `symbi-sandbox-guest` y
`symbi-sandbox-supervisor` se distribuyen en este repositorio, de modo que
puedes construir, auditar y reproducir la imagen del invitado por tu cuenta.

La referencia vigente es la [guia en ingles](/firecracker-setup). Describe el
aprovisionamiento de los artefactos, el `symbi-sandbox-guest` correspondiente como
PID 1 del invitado, el kernel y el rootfs compatibles, y el protocolo vsock
limitado en su version 5 con los modos oneshot, stdio y PTY. No se transfiere
automaticamente un directorio del host a la VM; la consola serie no proporciona
los resultados de las herramientas. Las imagenes de invitado obsoletas se rechazan
por su huella antes de enviar ningun comando.

Las operaciones fijas `read_file`, `list_files` y `grep_files` usan `source_roots`
de solo lectura explicitas a traves del broker de archivos acotado del runtime;
esas raices nunca se convierten en montajes del invitado ni en importaciones
automaticas. Los archivos declarados de comando, MCP y PTY usan transferencias de
bytes acotadas del protocolo 5 y un techo aparte, `output_roots`, para las salidas
nuevas en el host. Consulta [consultas de codigo fuente](/source-queries) y
[concesiones de archivos](/filesystem-grants#firecracker-file-transfer). Git usa un
[flujo de instantaneas acotado](/git-source-queries#firecracker-snapshots) aparte,
con un sistema de archivos del invitado sellado.

La ejecucion aislada de navegador sigue sin estar disponible; las rutas no
soportadas que se seleccionen fallan de forma explicita. El HTTP nativo sigue
siendo una operacion del broker del host, y los parsers personalizados se envian a
la VM seleccionada. Consulta [aislamiento de comandos](/toolclad-command-boundary)
y [cobertura de la rama](/containment-branch-guide).

En la [ejecucion de CLI administrada](/managed-cli-containment), el runtime emite
exactamente dos capacidades del invitado hacia el host: en CID 2, el puerto 4051
llega al intermediario de herramientas gobernado de esa ejecucion y el puerto 4052
a su intermediario de inferencia protegido. Los demas puertos no tienen endpoint, y
la configuracion del proyecto no puede seleccionar esas capacidades de socket.

El supervisor normal se ejecuta con la cuenta del usuario y reserva CPU y memoria
del invitado en el [grupo compartido](/shared-budgets). No configura jailer,
cgroups del host ni reservas adicionales para el VMM.

El [servicio administrado opcional](/firecracker-host-service) incorpora
artefactos aprobados, jailer, identidades separadas para los VMM, cgroups y
reservas de memoria que incluyen el gasto adicional del VMM. systemd supervisa
el servicio y la limpieza. `service_uid = 0` exige ese servicio; si no esta
disponible, no se inicia un sustituto local. Docker/gVisor necesitan una
asignacion de capacidad separada. No se proporciona un intermediario de destinos
de red.

La compilacion, las pruebas especificas y el E2E de regresion de Docker pasan.
El E2E privilegiado del host con KVM, fallos del servicio y watchdog sigue
pendiente. Este perfil requiere una validacion satisfactoria en el host de
destino. La imagen de prueba y los casos de regresion no acreditan una contencion
completa. La [guia del servicio](/firecracker-host-service) incluye los
comandos de provisionamiento y prueba.
