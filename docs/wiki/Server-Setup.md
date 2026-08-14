# Server aufsetzen

Diese Seite beschreibt einen typischen Start fuer eine selbst gehostete Singularis-Instanz.

## Voraussetzungen

- Linux-Server mit aktuellem Systemstand
- Domainname, wenn der Server oeffentlich erreichbar sein soll
- Docker oder Podman
- Git
- TLS-Zertifikate bzw. Reverse Proxy fuer oeffentlichen Betrieb

## Grober Ablauf

1. Repository klonen.
2. Konfiguration aus `env.example` anpassen.
3. Datenvolumes fuer Datenbank, MinIO und andere Dienste vorbereiten.
4. Services lokal starten.
5. Logausgabe und Healthchecks pruefen.

## Typische Startkommandos

```bash
git clone https://github.com/DNSx64/Singularis.git
cd Singularis
cp env.example .env
```

Danach die Umgebungsvariablen anpassen und die gewaehlte Laufzeit starten, zum Beispiel mit Docker Compose oder den projektspezifischen Startanweisungen aus dem Repository.

## Was zu pruefen ist

- Bindet der Server nur an die gewuenschte Adresse?
- Sind die Datenbank- und Objektspeicher-Volumes vorhanden?
- Laeuft der Relay-Dienst sauber hoch?
- Stimmen die Logs ohne Klartextdaten?
