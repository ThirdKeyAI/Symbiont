# Optionale ausgehende Zugriffe

Diese mit 3.0 eingeführte Funktion ist ausdrücklich optional. Ein
betreibereigenes `[outbound]`-Profil begrenzt jedes verwaltete ToolClad-HTTP-Werkzeug
auf eine benannte Operation mit festem Ursprung, HTTP-Methode, Pfadgrenze und
Anfragelimit. Ohne Profil bleibt das bisherige Verhalten erhalten.

Zugangsdaten stammen ausschließlich aus der ausdrücklich gewählten
Umgebungsvariable oder dem konfigurierten Secret Store. Ein Store-Fehler führt
nicht zu einem Rückgriff auf Umgebungsvariablen. Die Runtime fügt das Geheimnis
erst beim Versand ein und blockiert Antworten, die dessen unveränderten Wert
enthalten. Dies ist kein allgemeiner Schutz gegen Datenabfluss.

Jede Ausführung erhält eine eigene widerrufbare Berechtigung. Abbruch verhindert
weitere Zulassungen; bereits empfangene Anfragen können beim Ziel dennoch wirken.
Ungeklärte Wirkungen erfordern Prüfung und Abgleich statt automatischer Wiederholung.
Schreibzugriffe können die bestehende exakte Terminalgenehmigung verwenden.

Pfadparameter sind standardmäßig gesperrt. `allow_path_parameters = true`
erlaubt gewöhnliche Parameter, aber niemals Punktsegmente wie `..%3B`.
`allowed_query_keys` begrenzt optional die dekodierten Abfrageschlüssel und
verhindert doppelte Schlüssel. Zugangsdaten werden pro Ausführung erfolgreich
nur einmal aufgelöst, auch bei gleichzeitigen Aufrufen, und bei Widerruf aus dem
Cache entfernt. Fehlgeschlagene Auflösungen können erneut versucht werden.
Journal-Hashes enthalten keine Vault-Token oder AppRole-Secret-IDs; reflektierte
Geheimnisse in Antwort-Headernamen und -werten werden vor dem Hashen abgewiesen.
Die deklarierte `version` kennzeichnet eine beabsichtigte Rotation.

Vault authentifiziert sich vor dem Lesen mit der konfigurierten Methode. Umgebungsvariablen für Vault-Token oder TLS überschreiben diese Einstellungen nicht. Die Reflexionsprüfung berücksichtigt kleingeschriebene Headernamen und entfernte Leerzeichen an den Rändern von Headerwerten.

`symbi tools validate` und `symbi doctor` prüfen die Konfiguration, ohne das
Anwendungsgeheimnis abzurufen. Die Durchsetzung bleibt OSS. Für bestehende
Linux-Landlock-Installationen ist kein zusätzlicher Proxy, Container oder eine VM
erforderlich. Andere Transportwege und Browser-Ausführung werden dadurch nicht
abgedeckt; die gespeicherten CLI-Nachweise qualifizieren keine Sandbox.

Die [vollständige Konfiguration und Migration auf Englisch](/outbound-capabilities)
enthält TOML-Beispiele, Grenzen und Hinweise für Rust-Aufrufer. Der
[Kontrollkatalog](/security/README) zeigt Implementierungs- und Prüfstatus getrennt.

Rust-API-Migration: Ergänzen Sie in Struct-Literalen `HttpDef.capability: None`, `InferenceBrokerConfig.credential: None` und `ToolEffect::NetworkRequestStarted.outbound: None`. `SecurityTier` hat keine totale Ordnung (`Ord`) mehr. Verwenden Sie `satisfies` für Mindestanforderungen; Landlock erfüllt keine Container-/VM-Anforderung und umgekehrt.
