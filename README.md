# oranc

OCI Registry As Nix Cache.

Use an OCI registry (typically, [ghcr.io](https://ghcr.io)) to distribute binary caches of your Nix packages!

## Warning

1. Tags, image manifests, and layers created by oranc are so different from other typical OCI repositories.
So I don't know if it is an abuse of OCI registries. Pushing to [ghcr.io](https://ghcr.io) may violate the terms of service of GitHub.

2. Repository schema of oranc is still unstable.

   Tag encoding has been updated to support CA realisations. To use old pushed cache, please use the `--fallback-encodings base32-dnssec` option.

   ```console
   $ oranc tag encode "realisations/sha256:67890e0958e5d1a2944a3389151472a9acde025c7812f68381a7eef0d82152d1!libgcc.doi"
   realisations_L_sha256_W_67890e0958e5d1a2944a3389151472a9acde025c7812f68381a7eef0d82152d1_x_libgcc.doi
   $ oranc tag encode "realisations/sha256:67890e0958e5d1a2944a3389151472a9acde025c7812f68381a7eef0d82152d1!libgcc.doi" \
       --fallbacks --fallback-encodings base32-dnssec
   realisations_L_sha256_W_67890e0958e5d1a2944a3389151472a9acde025c7812f68381a7eef0d82152d1_x_libgcc.doi
   e9im2r39edgn8qbfdppiusr8c4p3adhq6orjge9gcko3id9ockqm8cb168sj8d316cpjge9h6koj8dpic4sm2or4cko34db36ss32cj66os36e1hc4rmapb661i3gchh6kp68c91dhkm4pr3ccn68rr9
   ```

   The `base32-dnssec` encoding for realisation is too long to fit into an OCI reference tag.

## Usage

### Push to OCI registry

There are two different ways to push cache to OCI registry using oranc.

* Push using `nix copy` with oranc server.
* Direct push using `oranc push` (faster but has some limitations).

#### Push with nix copy and oranc server

1. Host a oranc server, or try [oranc.li7g.com](https://oranc.li7g.com). It's better to self-host an instance. If you do so, please replace all `oranc.li7g.com` below with your instance.

   ```bash
   oranc server --listen "{LISTEN_ADDRESS}:{LISTEN_PORT}"
   ```

2. Set your credentials.

   ```bash
   export ORANC_USERNAME={YOUR_OCI_REGISTRY_USERNAME}
   export ORANC_PASSWORD={YOUR_OCI_REGISTRY_PASSWORD}
   export AWS_ACCESS_KEY_ID=$(echo -n "$ORANC_USERNAME:$ORANC_PASSWORD" | base64 --wrap 0)
   export AWS_SECRET_ACCESS_KEY="_"
   ```

3. Build something.

   ```bash
   nix build
   ```

4. Push to your OCI registry.

   ```bash
   nix copy --to "s3://{OCI_REPOSITORY_PART2}?endpoint=https://oranc.li7g.com/{OCI_REGISTRY}/{OCI_REPOSITORY_PART1}" ./result
   ```

   Cache will be pushed to `https://{OCI_REGISTRY}/{OCI_REPOSITORY_PART1}/{OCI_REPOSITORY_PART2}`.

#### Direct push

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

#### Limitations

1. `oranc push` reads the SQLite database `/nix/var/nix/db/db.sqlite`. The directory containing the database, `/nix/var/nix/db`, is typically owned by root. To open the database, `oranc` must have permission to create WAL files under the directory.

   To avoid requiring root permission to do `oranc push`, if `oranc push` does not able to create files under `/nix/var/nix/db/db.sqlite` and the argument `--allow-immutable-db` is passed, it will open the database in `immutable=1` mode, if another process writes to the database, `oranc push --allow-immutable-db` may fail.

2. `oranc push` does not support pushing content-addressed realisations.

### Use OCI registries as substituters

Try [oranc.li7g.com](https://oranc.li7g.com). It's better to self-host an instance. If you do so, please replace all `oranc.li7g.com` below with your instance.

Add settings to `nix.conf`:

```text
substituters = https://oranc.li7g.com/{OCI_REGISTRY}/{OCI_REPOSITORY}
trusted-public-keys = {PUBLIC_KEY}
```

or use NixOS configuration:

```nix
{ ... }:
{
  nix.settings = {
    substituters = [ "https://oranc.li7g.com/{OCI_REGISTRY}/{OCI_REPOSITORY}" ];
    trusted-public-keys = [ "{PUBLIC_KEY}" ];
  };
}
```

If your OCI registry requires authentication, HTTP basic authentication is supported:

1. Add username and password to the substituter URL: `https://{ORANC_USERNAME}:{ORANC_PASSWORD}@oranc.li7g.com/{OCI_REGISTRY}/{OCI_REPOSITORY}`.
2. Or use a netrc file <https://nixos.wiki/wiki/Enterprise>.

**Your credential will be sent to the oranc server.** If you don't trust my instance, please host your own instance.

## Host oranc server

Simply run,

```bash
oranc server --listen "{LISTEN_ADDRESS}:{LISTEN_PORT}"
```

Run `oranc server --help` for more options.

A NixOS module (`github:linyinfeng/oranc#nixosModules.oranc`) and a nixpkgs overlay (`github:linyinfeng/oranc#overlays.oranc`) are provided.

## TODO

[ ] Improve push performance of `oranc server`.
