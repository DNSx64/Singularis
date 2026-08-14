# Expose Server Publicly

If a Singularis instance should be reachable from the internet, network, TLS, and proxy setup must be explicit and hardened.

## Minimum requirements

- Public domain
- TLS certificate
- Reverse proxy in front of internal services
- Firewall rules for required ports only
- Strict controls for rate limits and logging

## Recommended steps

1. Point DNS records to your server.
2. Configure reverse proxy for HTTPS.
3. Expose only required public endpoints.
4. Bind internal services to 127.0.0.1 or private networks.
5. Verify external access and error handling.

## Security checklist

- No admin ports exposed publicly.
- No plaintext content in proxy logs.
- Backup and restore path is documented and tested.
- Host and container updates are applied regularly.
- Monitoring exists for TTL behavior, errors, and storage pressure.
