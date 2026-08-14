# Server oeffentlich machen

Wenn eine Singularis-Instanz oeffentlich erreichbar sein soll, sollten Netzwerk, TLS und Proxy sauber eingerichtet sein.

## Mindestanforderungen

- Oeffentliche Domain
- TLS-Zertifikat
- Reverse Proxy vor den internen Diensten
- Firewall nur fuer die benoetigten Ports
- Harter Blick auf Rate Limits und Logging

## Empfohlene Schritte

1. DNS-Eintrag auf den Server zeigen lassen.
2. Reverse Proxy fuer HTTPS konfigurieren.
3. Nur die oeffentlichen Endpunkte freigeben.
4. Interne Services auf `127.0.0.1` oder private Netze binden.
5. Zugriff und Fehlermeldungen pruefen.

## Sicherheitscheckliste

- Keine offenen Adminports im Internet.
- Kein Klartext in Proxy-Logs.
- Backup- und Restore-Pfad dokumentiert.
- Regelmaessige Updates fuer Host und Container.
- Monitoring fuer TTL, Fehler und Speicherverbrauch.
