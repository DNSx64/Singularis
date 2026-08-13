# Singularis Implementierungs-Roadmap (Solo)

Stand: 2026-08-13  
Teamgroesse: 1 Entwickler  
Zeithorizont: 18 Wochen (6 Sprints a 3 Wochen)

## Zielbild

Diese Roadmap priorisiert zuerst alle Blocker, die laut Spezifikation Phase 1 stoppen, und fuehrt danach in Richtung Private Alpha.

Prioritaet 1:
- Offene ADRs 2, 3, 5, 7, 11 schliessen
- Folgearbeiten aus ADR 0001 und ADR 0004 adressieren
- Vertikalen E2EE + TTL + Vault-Datenfluss stabilisieren und reproduzierbar pruefen

Prioritaet 2:
- Multi-User/Multi-Device Grundfunktionen
- Rollen/Einladungen/Recovery-Flows
- Datei-Flow mit Ablaufregeln

Prioritaet 3:
- Produktionsreife (Release-Sicherheit, reproduzierbare Artefakte, Runbooks)

## Solo-Arbeitsmodus

Pro Sprint Zeitbudget:
- 60 Prozent Implementierung
- 25 Prozent Tests und Debugging
- 15 Prozent ADR, Doku und Runbooks

WIP-Limit:
- Maximal 1 grosses Feature gleichzeitig
- Maximal 1 Nebenbaustelle gleichzeitig

Merge-Regel:
- Keine neuen Feature-Branches ohne gruenen Security- und Regressionstestlauf

## Nicht Teil dieser 18-Wochen-Roadmap

- Foederation
- Schluesseltransparenz-Log als produktionsreifes System
- Android/iOS produktionsreif
- E2EE Sprach- und Videochat
- Decoy-Vault

## Sprintplan

## Sprint 1 (Woche 1-3): Architektur-Blocker schliessen I

Scope:
- ADR 0002: Format und Signaturmodell fuer Geraeteberechtigungen
- ADR 0005: Binaeres Eventformat und Versionierungsstrategie

Konkrete Deliverables:
- Signierte Device-Grant-Struktur mit Version, Ablauf, Widerruf und Audit-Feldern
- Einheitliche Event-Huelle (Header, Schema-Version, Sequenzbezug, Replay-Felder)
- Forward/Backward-Kompatibilitaetsregeln dokumentiert
- Testvektoren fuer Device-Grant-Verifikation und Event-Parser

Definition of Done:
- ADR 0002 und ADR 0005 als accepted markiert
- Parser/Verifier in CI mit positiven und negativen Testfaellen
- Keine stillen Downgrades moeglich

## Sprint 2 (Woche 4-6): Architektur-Blocker schliessen II

Scope:
- ADR 0003: Event-Spool, Replikation, WAL-Regeln, messbares Loeschfenster
- ADR 0011: Vault-Exportformat, KDF, Streaming-AEAD, Testvektoren

Konkrete Deliverables:
- Exakte Ablauf-Invarianten fuer accepted_at/expires_at und Replikation
- Nachweisbare Loeschlatenz-Metrik und Alarmgrenzen
- Vault-Containerformat v1 mit Manifest + Chunk-Authentisierung
- Fehlerverhalten fuer falsches Passwort/Manipulation vereinheitlicht

Definition of Done:
- ADR 0003 und ADR 0011 accepted
- Reproduzierbarer TTL-Robustheitstest bei Restart/Worker-Fehlern
- Export/Import-Tests fuer korrupte Chunks und Manifest-Tampering

## Sprint 3 (Woche 7-9): Architektur-Blocker schliessen III + Release-Sicherheit

Scope:
- ADR 0007: Sicherer Updatepfad und Release-Schluesselverwaltung
- Folgepunkte aus ADR 0001 und ADR 0004 fuer Produktionsnaehe

Konkrete Deliverables:
- Signaturkette fuer Releases, Schluesselrollen und Rotation dokumentiert
- CI-Haertung: reproduzierbare Kernbuilds + Provenienz-Checks
- SQLCipher/Keyring-Entscheidung fuer Linux-Releasepfad (mind. Referenzpfad)
- OpenMLS Upgrade- und Regression-Gates verbindlich in CI

Definition of Done:
- ADR 0007 accepted
- Rebuild verifiziert identische Artefakte fuer definierte Targets
- Security-Gates blockieren Release bei ungeklaerten Binardifferenzen

## Sprint 4 (Woche 10-12): Multi-Device Fundament und Rollen-Basis

Scope:
- Multi-User/Multi-Device Provisioning-Basis
- Rollen- und Kanalberechtigungen MVP-Basis

Konkrete Deliverables:
- Device-Pairing-Protokoll im Prototyp (QR + Compare-Code + Signaturkette)
- Widerrufspfad mit MLS-Epochenwechsel als Pflichtfluss
- Rollenmodell Owner/Admin/Moderator/Member/Guest serverseitig durchgesetzt
- Basis-API fuer Einladungen inklusive Ablauf und einmaliger Nutzung

Definition of Done:
- Nicht autorisierte oder erfundene Geraete koennen nicht beitreten
- Rollenverletzende Events werden server- und clientseitig verworfen
- Integrationssuite mit 2 Benutzern und je 2 Geraeten stabil

## Sprint 5 (Woche 13-15): Recovery, Datei-Flow, Moderations-Grundlagen

Scope:
- Recovery-Ende-zu-Ende fuer Identitaet
- Verschluesselter Datei-Flow mit RESERVED/COMMITTED/EXPIRED
- Rate-Limits und Meldungs-Grundablauf

Konkrete Deliverables:
- Recovery-Export/Import UX + kryptographische Pruefungen
- Dateiupload mit harter Bindung an Event-ID und TTL-Kopplung
- Opake Loeschberechtigung fuer fruehes Entfernen serverseitiger Ciphertexte
- Token-Bucket-Limits fuer Login/Message/Upload/Invite/Report

Definition of Done:
- Recovery ohne Server-Schluesselzugriff reproduzierbar erfolgreich
- Objekt-TTL nie laenger als Event-TTL
- Moderationsbericht offenbart nur explizit ausgewaehlte Inhalte

## Sprint 6 (Woche 16-18): Private-Alpha-Haertung

Scope:
- Stabilisierungsphase fuer Private Alpha
- Sicherheits-, Last- und Betriebsnachweise

Konkrete Deliverables:
- Canary- und Leak-Tests fuer DB, Object Store, Logs, Traces, Backups
- Migrations-, Replay-, Tamper-, Restart- und Clock-Drift-Regressionen
- Self-Hosting-Dokumentation fuer Referenzbetrieb (Single-Instance)
- Incident-Runbooks und operative Mindestmetriken

Definition of Done:
- Kritische Abnahmekriterien fuer Alpha nachweisbar erfuellt
- Keine bekannten P0/P1 Security Findings offen
- Kleine Test-Community laeuft mehrere Wochen ohne Reset des Datenmodells

## Abhaengigkeiten und Reihenfolge

Blockierende Reihenfolge:
1. ADR 0002
2. ADR 0005
3. ADR 0003
4. ADR 0011
5. ADR 0007

Hinweis:
- Ohne diese Reihenfolge steigt das Risiko von Rueckbau in Protokoll, Persistenz und Release-Prozess deutlich.

## KPI-Set pro Sprint

Technische KPIs:
- Anteil bestandener Security-Regressionstests
- Mittlere Loeschlatenz und P95-Loeschlatenz
- Erfolgsrate Recovery-Tests
- Reproduzierbarkeit der Release-Artefakte
- Anzahl offener kritischer Findings

Produkt-KPIs:
- Erfolgsrate Device-Pairing ohne manuellen Eingriff
- Erfolgsrate Outbox-Wiederaufnahme nach Crash/Netzfehler
- Verstaendlichkeit von TTL vs. lokaler Aufbewahrung in Nutzertests

## Risiken und Gegenmassnahmen

Risiko: Ueberlastung durch zu viele parallele Themen  
Gegenmassnahme: Striktes WIP-Limit und pro Sprint nur ein Hauptfeature

Risiko: Architekturentscheidungen werden zu spaet finalisiert  
Gegenmassnahme: ADR-Deadline pro Sprint mit expliziter Eskalation nach 3 Werktagen Verzug

Risiko: Feature-Entwicklung ueberholt Sicherheits-Gates  
Gegenmassnahme: Release-Branch darf nur bei gruener Security-Suite erstellt werden

Risiko: Kryptographie- oder Speicherpfade aendern sich kurz vor Alpha  
Gegenmassnahme: Strict Change Freeze ab Mitte Sprint 6, nur Fixes

## Konkrete naechste Schritte ab heute

1. ADR 0002 direkt als first draft ausformulieren und in einer Session finalisieren
2. CI-Matrix fuer Parser-/Verifier-Negativtests aufsetzen
3. Testkatalog fuer TTL-Robustheit und Export-Tampering als verbindliche Checkliste anlegen
4. Pairing-Prototyp API-Schnitt dokumentieren, bevor UI-Flow erweitert wird
