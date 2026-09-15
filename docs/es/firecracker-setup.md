---
layout: default
title: Configuracion de Firecracker (Nivel 3)
nav_order: 8
nav_exclude: true
---

# Configuracion de Firecracker (Nivel 3)

Firecracker ejecuta las cargas en una microVM con su propio kernel invitado.
El kernel del host, el VMM, la imagen y la configuracion del operador siguen
siendo componentes de confianza. El aislamiento no garantiza que todo escape
sea imposible.

El Nivel 3 forma parte del runtime de codigo abierto. No requiere clave de
licencia ni compilacion Enterprise: los crates `symbi-sandbox-guest` y
`symbi-sandbox-supervisor` se distribuyen en este repositorio, de modo que
puedes construir, auditar y reproducir la imagen del invitado por tu cuenta.

La referencia actual es la [guia en ingles](../firecracker-setup.md). Describe
el `symbi-sandbox-guest`, kernel y rootfs compatibles y el protocolo vsock
limitado. No se transfiere automaticamente un directorio del host a la VM;
la consola serie no proporciona los resultados de las herramientas.

El supervisor normal se ejecuta con la cuenta del usuario y reserva CPU y memoria
del invitado en el [grupo compartido](../shared-budgets.md). No configura jailer,
cgroups del host ni reservas adicionales para el VMM.

El [servicio administrado opcional](../firecracker-host-service.md) incorpora
artefactos aprobados, jailer, identidades separadas para los VMM, cgroups y
reservas de memoria que incluyen el gasto adicional del VMM. systemd supervisa
el servicio y la limpieza. `service_uid = 0` exige ese servicio; si no esta
disponible, no se inicia un sustituto local. Docker/gVisor necesitan una
asignacion de capacidad separada.

La compilacion, las pruebas especificas y el E2E de regresion de Docker pasan.
El E2E privilegiado del host con KVM, fallos del servicio y watchdog sigue
pendiente. Este perfil requiere una validacion satisfactoria en el host de
destino. La [guia del servicio](../firecracker-host-service.md) incluye los
comandos de provisionamiento y prueba.
