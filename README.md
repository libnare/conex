# Conex - container registry proxy

## Docker Image
```
cr.libnare.net/conex/main:latest
```
The `cr.libnare.net` is being run via [Cloud Run](https://cloud.google.com/run) using [Conex](https://github.com/libnare/conex).

## Environment Variables
- `BIND_HOST`: The server's binding address. Default is `0.0.0.0`.
- `BIND_PORT`: The port to which the server binds. Default is `8080`.
- `HOSTNAME`: The hostname used for actual access. It is typically used when the returned address should be fixed.
- `REGISTRY_HOST`: The host address of the target registry to be proxied.
- `REGISTRY_PREFIX`: (Required) The prefix of the target registry to be proxied.

## Authentication for private registries
Conex supports authentication for private registries. To enable authentication, set the following environment variables.

### `GOOGLE_APPLICATION_CREDENTIALS`
This option is used for Google Cloud. (Artifact Registry)<br>
Specify the path to the service account key file. For generating a service account key, see the following article: [keys-create-delete](https://cloud.google.com/iam/docs/keys-create-delete#iam-service-account-keys-create-console)
.
### `AUTH_HEADER`
This option is used for other registries.<br>
Use the value of `auth` in `~/.docker/config.json` after logging into Docker.
