# Deployment templates

Ready-made templates for running the published tidl image (backlog #152) on
NAS boxes and non-Docker container runtimes. All of them wrap the same
`ghcr.io` image and the same configuration surface (`TIDL_*` env vars, a `/data`
volume, and a secret file) — see the main [README](../README.md#deploy-with-docker)
and the [Docker Compose example](../docker-compose.yml) first.

| Target | Template | Notes |
|---|---|---|
| unRAID | [`unraid/tidl.xml`](unraid/tidl.xml) | Community Applications–style template |
| TrueNAS SCALE | [`truenas/compose.yaml`](truenas/compose.yaml) | Custom App (compose) |
| Podman (Quadlet) | [`podman/tidl.container`](podman/tidl.container) | systemd-native, rootless-friendly |

The image is published under the `puckles7` GitHub org. Create the secret:

```sh
tidl-server generate-secret            # writes ./tidl-secret
# or, equivalently: printf '%s' "$(openssl rand -base64 32)" > tidl-secret
```

## unRAID

Copy `unraid/tidl.xml` to your unRAID server under
`/boot/config/plugins/dockerMan/templates-user/`, then add the container from
**Docker → Add Container → Template**. The template maps the web UI port, the
`/data` appdata path, and the secret file. Set `TIDL_DOMAIN` to the hostname
your devices reach the server on.

## TrueNAS SCALE

TrueNAS SCALE 24.10+ can run a compose "Custom App". Put `truenas/compose.yaml`
in a dataset (e.g. `/mnt/pool/apps/tidl/compose.yaml`) with the `tidl-secret`
file alongside it, then **Apps → Discover → Custom App** and paste the YAML (or
point at the file). The volume path in that template assumes
`/mnt/pool/apps/tidl` — adjust it.

## Podman / rootless

The image is a plain OCI image, so Podman runs it directly:

```sh
podman run -d --name tidl \
  -p 3060:3060 \
  -v tidl-data:/data \
  -e TIDL_SECRET_FILE=/run/secrets/tidl-secret \
  --secret tidl-secret \
  ghcr.io/puckles7/tidl:latest
```

For a persistent, systemd-managed service, use the Quadlet template
(`podman/tidl.container`) instead — Quadlet turns it into a real systemd unit:

```sh
podman secret create tidl-secret ./tidl-secret
mkdir -p ~/.config/containers/systemd
cp podman/tidl.container ~/.config/containers/systemd/
systemctl --user daemon-reload
systemctl --user start tidl
```

Rootless Podman cannot bind ports below 1024 by default; 3060 is fine. To reach
the server from other machines, publish on `0.0.0.0` (as the template does) and
open the port in your firewall.
