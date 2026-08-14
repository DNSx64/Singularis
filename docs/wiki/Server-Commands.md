# Nützliche Server-Kommandos

Die folgenden Kommandos sind typische Helfer fuer Betrieb und Fehlersuche.

## System

```bash
uname -a
uptime
df -h
free -h
```

## Dienste und Logs

```bash
systemctl status singularis
journalctl -u singularis -f
```

## Containerbetrieb

```bash
docker ps
docker compose ps
docker compose logs -f
docker compose restart
```

## Netzwerk

```bash
ss -tulpn
curl -I https://example.com
```

## Wartung

```bash
git pull
docker compose up -d --build
docker image prune
```
