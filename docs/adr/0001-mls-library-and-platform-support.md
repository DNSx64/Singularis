# ADR 0001: MLS-Bibliothek und Plattformunterstuetzung

- Status: Fuer den Phase-1-Desktop-Prototyp akzeptiert
- Datum: 2026-08-09
- Entscheidungstraeger: Singularis-Kernteam

## Kontext

Singularis benoetigt eine RFC-9420-konforme Gruppenverschluesselung fuer Direkt- und Gruppennachrichten. Eine eigene Konstruktion ist ausgeschlossen. Die Bibliothek muss auf den nativen Desktop-Zielplattformen funktionieren, einen spaeteren Browserpfad nicht unnoetig verhindern und einen nachvollziehbaren Umgang mit Sicherheitsmeldungen besitzen.

## Entscheidung

1. Der Phase-1-Desktop-Prototyp verwendet die OpenMLS-`0.8.1`-API-Linie unter der MIT-Lizenz. `openmls`, `openmls_basic_credential` und `openmls_rust_crypto` werden gemeinsam auf den Git-Commit `aefca1c182feb431e86fb9fcc74d912f4688639c` gepinnt. Dieser Stand meldet weiterhin die Paketversionen `0.8.1`, `0.5.0` und `0.5.1`.
2. Die feste erste Cipher Suite ist `MLS_128_DHKEMX25519_CHACHA20POLY1305_SHA256_Ed25519`.
3. Anwendungs- und Handshake-Nachrichten verwenden ein reines MLS-Ciphertext-Wire-Format. Der Server erhaelt serialisierte MLS-Nachrichten nur als opake Bytes und fuehrt keine eigene Inhaltsverschluesselung aus.
4. Eine `BasicCredential` enthaelt im Prototyp ausschliesslich eine opake Geraete-ID. Vor einer echten Geraeteaufnahme muss deren Bindung an eine signierte Geraeteberechtigung durch ADR 0002 festgelegt werden.
5. OpenMLS-Zustand und Signaturschluessel duerfen in Produktbuilds nicht im ungeschuetzten Dateisystem liegen. `VaultMlsClient` serialisiert den vollstaendigen OpenMLS-Providerzustand zusammen mit den Singularis-Senderketten als begrenzten, versionierten Snapshot. Ausgehende Anwendungsnachrichten werden als kanonischer `SubmitEvent` gemeinsam mit ihrem Nachfolge-Snapshot in einer SQLCipher-Transaktion abgelegt. Jede zustandsaendernde Operation gibt KeyPackage, Welcome, Ciphertext oder Klartext erst nach ihrem Checkpoint zurueck. Schlaegt eine Operation oder der Checkpoint fehl, wird der vorherige Snapshot im Speicher wiederhergestellt. Temporaere serialisierte Snapshot-Puffer werden nach Schreiben, Laden und Rollback ueberschrieben. Der direkte `MlsClient` mit In-Memory-Provider bleibt nur fuer isolierte Kryptotests zulaessig.
6. Die Browserunterstuetzung ist noch nicht akzeptiert. OpenMLS `0.8.1` besitzt ein `js`-Feature und wird fuer `wasm32-unknown-unknown` gebaut, Upstream fuehrt dieses Ziel jedoch als nicht unterstuetzt und nicht getestet. Vor Phase 3 sind Browser-Interop, Zufallsquelle, Speicherloeschung, Bundlegroesse und fluechtige Schluesselhaltung separat zu pruefen.
7. Abhaengigkeitsupdates erfolgen nicht automatisch ueber eine kompatible Versionsspanne. Jede neue OpenMLS-Version benoetigt Interop-, Manipulations-, Replay-, Epochen-, Persistenz- und Migrationspruefungen.

## Sicherheits- und Wartungsstatus

OpenMLS ist eine aktive RFC-9420-Implementierung mit oeffentlicher Security-Policy und privatem Meldeweg. Fuer die gewaehlte Linie sind zwei veroeffentlichte Advisories relevant:

- [GHSA-qr9h-x63w-vqfm](https://github.com/openmls/openmls/security/advisories/GHSA-qr9h-x63w-vqfm) betrifft Versionen bis einschliesslich `0.7.0` und ist ab `0.7.1` behoben.
- [GHSA-8x3w-qj7j-gqhf](https://github.com/openmls/openmls/security/advisories/GHSA-8x3w-qj7j-gqhf) betrifft Versionen vor `0.7.2`; der Fix ist auch in `0.8.0` und damit in `0.8.1` enthalten.

Zum Entscheidungsdatum ist im Upstream-Repository, in den Release Notes und in der Security-Policy kein verlinkter unabhaengiger Gesamtaudit auffindbar. OpenMLS wird deshalb nicht als auditiert bezeichnet. Vor einer stabilen Freigabe sind mindestens eine dokumentierte externe Pruefung des eingesetzten Versionsstands und eine Bewertung des Singularis-Integrationscodes erforderlich.

OpenMLS `0.8.1` enthielt ausserdem `debug_assert!(false)` in Fehlerpfaden fuer manipulierte private Nachrichten und Welcome-Daten. Dadurch konnte untrusted Input in Debug-Builds paniken, obwohl die API einen Fehlerwert versprach. Upstream dokumentierte dies als [Issue #1998](https://github.com/openmls/openmls/issues/1998) und entfernte die Assertions mit dem gemergten [PR #2001](https://github.com/openmls/openmls/pull/2001). Da noch kein entsprechendes stabiles `0.8.x`-Patchrelease vorliegt, nutzt Singularis den exakten, GitHub-verifizierten Fix-Commit `aefca1c182feb431e86fb9fcc74d912f4688639c` fuer alle OpenMLS-Crates. Ein Manipulationstest muss bei jedem Pin-Wechsel beweisen, dass der Fehler als `Err` zurueckkehrt und der letzte persistierte Ratchet-Zustand wiederhergestellt wird.

Die Kennung `GHSA-rrmv-c79f-cf5r`, die in unreleased Upstream-Unterlagen im Zusammenhang mit manuellen Decodern referenziert wurde, ist zum Entscheidungsdatum weder ueber GitHubs oeffentliche Advisory-Seite beziehungsweise API noch ueber RustSec aufloesbar. Betroffene Versionen und Relevanz fuer den Pin sind daher nicht verifizierbar. Die Kennung bleibt ein offener Security-Reviewpunkt und muss vor einer Produktionsfreigabe mit Upstream oder einer veroeffentlichten Advisory-Fassung geklaert werden; aus der nicht aufloesbaren Referenz wird weder Entwarnung noch eine bestaetigte Verwundbarkeit abgeleitet.

Das Snapshot-Format enthaelt die internen Schluessel und Werte des gepinnten OpenMLS-Memory-Stores. Es ist deshalb an den exakten OpenMLS-Stand gebunden und besitzt eine eigene Formatversion. Ein Pin-Wechsel erfordert neben den Protokolltests eine explizite Snapshot-Migration oder eine kontrollierte Neuaufnahme der betroffenen Gruppen. Neustarttests beweisen derzeit die Fortsetzung von Ratchet, Senderzaehler, Hash-Kette und bidirektionaler Kommunikation sowie die Wiederherstellung nach manipuliertem Input. Der erste Outbox-Integrationstest beendet Alice nach der Verschluesselung, laedt den Auftrag erneut, sendet ihn zweimal idempotent an Axum, laesst Bob den einmal gespeicherten Event entschluesseln und quittiert ihn erst danach lokal.

Der Workspace deklariert Rust `1.85`. Der Stack baut im aktuellen Void-Linux-Entwicklungssystem mit Rust `1.97.1`; ein Rust-`1.85.0`-Toolchain ist dort nicht installiert. Ein eigener CI-Job mit exakt `1.85.0` ist daher ein offenes Freigabekriterium.

## Konsequenzen

- Der Kryptokern nutzt einen Standard mit Forward Secrecy und Post-Compromise Security statt einer selbst entworfenen Gruppenkonstruktion.
- Die exakten Pins verhindern unbemerkte Protokoll- oder Speicherformatwechsel, erfordern aber bewusst geplante Sicherheitsupdates.
- Der RustCrypto-Provider vergroessert den Abhaengigkeitsgraphen und ist Teil der zu pruefenden Lieferkette.
- MLS authentifiziert Gruppenmitglieder, loest aber weder Geraeteautorisierung, Serverauthentifizierung, Metadatenminimierung noch Split-View-Erkennung allein.
- Eine erfolgreiche persistente Zwei-Client-Demo ist kein Produktionsnachweis. Der produktive Sende-Worker, authentisierte Serverquittungen, Recovery, Credential-Bindung und ein externer Audit bleiben eigenstaendige Anforderungen.

## Verworfene Alternativen

- Eigene Gruppen-AEAD oder Sender-Key-Konstruktion: liefert die geforderten MLS-Eigenschaften nicht verlaesslich und waere nicht vertretbar auditierbar.
- OpenMLS `0.9.0-rc.2`: ist eine Vorabversion; der Hauptzweig nennt Rust `1.91+` und passt damit nicht zur festgelegten Toolchain.
- Direkte Browserfreigabe aufgrund eines erfolgreichen Wasm-Builds: ein kompilierbares Ziel belegt weder sicheren Zufall noch Interoperabilitaet und fluechtige Schluesselhaltung.
- Ungepinnte semantische Versionsbereiche: koennen sicherheitsrelevante API- und Persistenzaenderungen ohne explizite Entscheidung einziehen.

## Folgearbeiten

1. `VaultMlsClient` und den Outbox-Sende-Worker in den Desktop-Lebenszyklus integrieren; direkte In-Memory-Clients in Produktpfaden ausschliessen.
2. Serverquittungen authentisieren und den Crash-Zeitpunkt zwischen erfolgreicher Annahme und lokaler Quittierung als idempotenten Retry testen.
3. Handshake-Auftraege wie KeyPackage, Commit und Welcome in ihre jeweiligen crashfesten Versandablaeufe integrieren.
4. Rust `1.85.0`, Desktop-Zielplattformen und spaeter Wasm in CI pruefen.
5. ADR 0002 fuer signierte Geraeteberechtigungen abschliessen.
6. Vor stabiler Freigabe externe Auditabdeckung dokumentieren.
7. Status und Versionsbereich von `GHSA-rrmv-c79f-cf5r` belastbar klaeren.