# Useful Server Commands

The commands below are common helpers for operations and troubleshooting.

## System

```bash
uname -a
uptime
df -h
free -h
```

## Services and logs

```bash
systemctl status singularis
journalctl -u singularis -f
```

## Container operations

```bash
docker ps
docker compose ps
docker compose logs -f
docker compose restart
```

## Network checks

```bash
ss -tulpn
curl -I https://example.com
```

## Maintenance

```bash
git pull
docker compose up -d --build
docker image prune
```
