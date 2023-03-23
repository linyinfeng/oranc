# oranc

OCI Registry As Nix Cache.

Use an OCI registry (typically, [ghcr.io](https://ghcr.io)) to distribute binary caches of your Nix packages!

## Usage

Try [oranc.li7g.com](https://oranc.li7g.com). It's better to self-host an instance. If you do so, please replace all `oranc.li7g.com` below with your instance.

1. Set your credentials.

   ```bash
   export ORANC_USERNAME={YOUR_OCI_REGISTRY_USERNAME}
   export ORANC_PASSWORD={YOUR_OCI_REGISTRY_PASSWORD}
   ```

2. Initialize your OCI registry.

   ```bash
   oranc push --registry {OCI_REGISTRY} --repository {OCI_REPOSITORY} initialize
   ```

   *Make the repository public*, otherwise, caching will not work.

3. Prepare your signing keys.

   ```console
   $ nix key generate-secret --key-name {KEY_NAME} > {PRIVATE_KEY_FILE}
   $ cat {PRIVATE_KEY_FILE} | nix key convert-secret-to-public
   {PUBLIC_KEY}
   ```

4. Build and sign something. oranc *only* pushes signed store paths.

   ```bash
   nix build
   nix store sign ./result --recursive --key-file {PRIVATE_KEY_FILE}
   ```

5. Push to your OCI registry.

   ```bash
   # you need to have write permission to `/nix/var/nix/db`
   # or pass the argument `--allow-immutable-db`
   # see Limitations
   echo ./result | oranc push --registry {OCI_REGISTRY} --repository {OCI_REPOSITORY}
   ```

   Run `oranc push --help` for more options.

6. Use your OCI registry as a cache.

   In `nix.conf`:

   ```text
   substituters = https://oranc.li7g.com/{OCI_REGISTRY}/{OCI_REPOSITORY}
   trusted-public-keys = {PUBLIC_KEY}
   ```

## Host oranc server

Simply run,

```bash
oranc server --listen "{LISTEN_ADDRESS}:{LISTEN_PORT}"
```

Run `oranc server --help` for more options.

## Limitations

`oranc push` reads the SQLite database `/nix/var/nix/db/db.sqlite`. The directory containing the database, `/nix/var/nix/db`, is typically owned by root. To open the database, `oranc` must have permission to create WAL files under the directory.

To avoid requiring root permission to do `oranc push`, if `oranc push` does not able to create files under `/nix/var/nix/db/db.sqlite` and the argument `--allow-immutable-db` is passed, it will open the database in `immutable=1` mode, if another process writes to the database, `oranc push --allow-immutable-db` may fail.
