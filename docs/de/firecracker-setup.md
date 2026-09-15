---
layout: default
title: Firecracker-Setup (Stufe 3)
nav_order: 8
nav_exclude: true
---

# Firecracker-Setup (Stufe 3)

Firecracker fuehrt Workloads in einer microVM mit eigenem Gast-Kernel aus.
Host-Kernel, VMM, Gast-Image und Betreiberkonfiguration bleiben vertrauenswuerdige
Komponenten. Die Isolation ist keine Garantie gegen alle Ausbruchswege.

Stufe 3 ist Teil des Open-Source-Runtimes. Sie benoetigt keinen
Lizenzschluessel und keinen Enterprise-Build: Die Crates `symbi-sandbox-guest`
und `symbi-sandbox-supervisor` sind in diesem Repository enthalten, sodass Sie
das Gast-Image selbst bauen, pruefen und reproduzieren koennen.

Die verbindliche, aktuelle Anleitung ist das
[englische Firecracker-Setup](../firecracker-setup.md). Es beschreibt den
passenden `symbi-sandbox-guest`, Kernel und Rootfs sowie das begrenzte
vsock-Protokoll. Ein Host-Arbeitsverzeichnis wird nicht automatisch in die VM
uebertragen; die serielle Konsole liefert keine Tool-Ergebnisse.

Der normale Supervisor laeuft unter dem Benutzerkonto und reserviert Gast-CPU
und Gast-Speicher im [gemeinsamen Pool](../shared-budgets.md). Er richtet weder
einen jailer noch Host-cgroups ein und reserviert keinen VMM-Zusatzspeicher.

Der optionale [verwaltete Host-Dienst](../firecracker-host-service.md) ergaenzt
gepruefte Artefakte, den jailer, separate VMM-Identitaeten, Host-cgroups und
Speicherreservierungen einschliesslich VMM-Aufschlag. systemd ueberwacht den
Dienst und die Bereinigung. `service_uid = 0` verlangt diesen Dienst; bei dessen
Ausfall wird kein lokaler Ersatz gestartet. Docker/gVisor brauchen eine separat
zugeteilte Kapazitaet.

Build, gezielte Tests und Docker-Regressions-E2E sind erfolgreich. Der privilegierte
Host-E2E mit KVM, Dienstabsturz und Watchdog ist noch ausstehend. Dieses Profil
gilt erst nach erfolgreicher Pruefung auf dem Zielhost als validiert. Die
[Host-Anleitung](../firecracker-host-service.md) enthaelt Provisionierung und Test.
