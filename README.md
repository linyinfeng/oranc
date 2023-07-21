# oranc

OCI Registry As Nix Cache.

Use an OCI registry (typically, [ghcr.io](https://ghcr.io)) to distribute binary caches of your Nix packages!

## Warning

Tags, image manifests, and layers created by oranc are so different from other typical OCI repositories.
So I don't know if it is an abuse of OCI registries. Pushing to [ghcr.io](https://ghcr.io) may violate the terms of service of GitHub.

## Usage

Try [oranc.li7g.com](https://oranc.li7g.com). It's better to self-host an instance. If you do so, please replace all `oranc.li7g.com` below with your instance.

1. Prepare your signing keys.

   ```console
   $ nix key generate-secret --key-name {KEY_NAME} > {PRIVATE_KEY_FILE}
   $ cat {PRIVATE_KEY_FILE} | nix key convert-secret-to-public
   {PUBLIC_KEY}
   ```

2. Set your credentials.

   ```bash
   export ORANC_USERNAME={YOUR_OCI_REGISTRY_USERNAME}
   export ORANC_PASSWORD={YOUR_OCI_REGISTRY_PASSWORD}
   export ORANC_SIGNING_KEY={YOUR_NIX_SIGNING_KEY}
   ```

3. Initialize your OCI registry.

   ```bash
   oranc push --registry {OCI_REGISTRY} --repository {OCI_REPOSITORY} initialize
   ```

   *Make the repository public*, otherwise, caching will not work.

4. Build something.

   ```bash
   nix build
   ```

5. Push to your OCI registry.

   ```bash
   # you need to have write permission to `/nix/var/nix/db`
   # or pass the argument `--allow-immutable-db`
   # see the Limitations section
   echo ./result | oranc push --registry {OCI_REGISTRY} --repository {OCI_REPOSITORY}
   ```

   `oranc` will sign the NAR archive on the fly using `ORANC_SIGNING_KEY`.

   Note that:

   1. only unsigned paths will be pushed, if you manually signed store paths, use the argument `--already-signed` to push them.
   2. Currently `oranc` will not sign local paths, run `... | xargs nix store sign --recursive --key-file {YOUR_KEY_FILE}` to sign paths locally.

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
