# Server Setup

This page describes a typical starting point for a self-hosted Singularis instance.

## Requirements

- A Linux server with current system updates
- A domain name if the server should be public
- Docker or Podman
- Git
- TLS certificates and a reverse proxy for public access

## High-level flow

1. Clone the repository.
2. Copy and adjust configuration from env.example.
3. Prepare data volumes for database, MinIO, and related services.
4. Start the services.
5. Verify logs and health checks.

## Typical bootstrap commands

```bash
git clone https://github.com/DNSx64/Singularis.git
cd Singularis
cp env.example .env
```

After that, adjust environment variables and start your selected runtime, for example with Docker Compose or the project-specific run instructions in the repository.

## What to verify

- Is the server bound only to the intended address?
- Are database and object-storage volumes present and writable?
- Does the relay service start cleanly?
- Are logs free of plaintext message content?
