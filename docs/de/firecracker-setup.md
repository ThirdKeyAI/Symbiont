---
layout: default
title: Firecracker-Setup (Stufe 3)
nav_order: 8
nav_exclude: true
---

# Firecracker-Setup (Stufe 3)

Stufe 3 fuehrt Oneshot-Kommandos, eigene Ausgabe-Parser, MCP-stdio-Server,
PTY-Sitzungen und verwaltete CLI-Worker in einer frischen Firecracker-microVM
aus. Die ausgewaehlte ToolClad-Grenze und der oeffentliche `FirecrackerRunner`
nutzen dasselbe Gast-Protokoll und denselben unabhaengigen Supervisor. Ein
VM-Start oder ein VMM-Ende kann das Ergebnis des angeforderten Kommandos nicht
ersetzen. Host-Kernel, VMM, Gast-Image und Betreiberkonfiguration bleiben
vertrauenswuerdige Komponenten. Die Isolation ist keine Garantie gegen alle
Ausbruchswege.

Stufe 3 ist Teil des Open-Source-Runtimes. Sie benoetigt keinen
Lizenzschluessel und keinen Enterprise-Build: Die Crates `symbi-sandbox-guest`
und `symbi-sandbox-supervisor` sind in diesem Repository enthalten, sodass Sie
das Gast-Image selbst bauen, pruefen und reproduzieren koennen.

Die verbindliche, aktuelle Anleitung ist das
[englische Firecracker-Setup](/firecracker-setup). Es beschreibt die
Provisionierung der Artefakte, den passenden `symbi-sandbox-guest` als Gast-PID 1,
Kernel und Rootfs sowie das begrenzte vsock-Protokoll in Version 5 mit den Modi
oneshot, stdio und PTY. Ein Host-Arbeitsverzeichnis wird nicht automatisch in die
VM uebertragen; die serielle Konsole liefert keine Tool-Ergebnisse. Veraltete
Gast-Images werden anhand des Fingerabdrucks abgelehnt, bevor ein Kommando
gesendet wird.

Feste `read_file`-, `list_files`- und `grep_files`-Operationen nutzen explizite,
schreibgeschuetzte `source_roots` ueber den begrenzten Datei-Broker des Runtimes;
diese Wurzeln werden nie zu Gast-Mounts oder automatischen Importen. Deklarierte
Dateien fuer Kommando, MCP und PTY verwenden begrenzte Byte-Uebertragungen des
Protokolls 5 und eine separate `output_roots`-Obergrenze fuer neue Host-Ausgaben.
Siehe [Quellabfragen](/source-queries) und
[Dateifreigaben](/filesystem-grants#firecracker-file-transfer). Git nutzt einen
eigenen [begrenzten Snapshot-Stream](/git-source-queries#firecracker-snapshots)
mit versiegeltem Gast-Dateisystem.

Isolierte Browser-Ausfuehrung bleibt nicht verfuegbar; nicht unterstuetzte Pfade
schlagen explizit fehl. Natives HTTP bleibt eine Operation des Host-Brokers,
eigene Parser werden in die ausgewaehlte VM gesendet. Siehe
[Kommando-Isolation](/toolclad-command-boundary) und
[Branch-Abdeckung](/containment-branch-guide).

Bei [verwalteter CLI-Ausfuehrung](/managed-cli-containment) vergibt das Runtime
genau zwei Gast-zu-Host-Faehigkeiten: CID 2, Port 4051 erreicht den gesteuerten
Tool-Broker dieses Laufs, Port 4052 seinen geschuetzten Inferenz-Broker. Andere
Ports haben keinen Endpunkt, und die Projektkonfiguration kann diese
Socket-Faehigkeiten nicht auswaehlen.

Der normale Supervisor laeuft unter dem Benutzerkonto und reserviert Gast-CPU
und Gast-Speicher im [gemeinsamen Pool](/shared-budgets). Er richtet weder
einen jailer noch Host-cgroups ein und reserviert keinen VMM-Zusatzspeicher.

Der optionale [verwaltete Host-Dienst](/firecracker-host-service) ergaenzt
gepruefte Artefakte, den jailer, separate VMM-Identitaeten, Host-cgroups und
Speicherreservierungen einschliesslich VMM-Aufschlag. systemd ueberwacht den
Dienst und die Bereinigung. `service_uid = 0` verlangt diesen Dienst; bei dessen
Ausfall wird kein lokaler Ersatz gestartet. Docker/gVisor brauchen eine separat
zugeteilte Kapazitaet. Ein Broker fuer Netzwerkziele wird nicht bereitgestellt.

Build, gezielte Tests und Docker-Regressions-E2E sind erfolgreich. Der privilegierte
Host-E2E mit KVM, Dienstabsturz und Watchdog ist noch ausstehend. Dieses Profil
gilt erst nach erfolgreicher Pruefung auf dem Zielhost als validiert. Das
Test-Image und die Regressionsfaelle belegen keine vollstaendige Eindaemmung. Die
[Host-Anleitung](/firecracker-host-service) enthaelt Provisionierung und Test.
