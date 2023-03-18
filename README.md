# oranc

OCI Registry As Nix Cache.

## Host

Simply run,

```bash
oranc --listen "{LISTEN_ADDRESS}:{LISTEN_PORT}"
```

Run `oranc --help` for more options.

## Usage

Try [oranc.li7g.com](https://oranc.li7g.com) and [oranc-upload.li7g.com](https://oranc-upload.li7g.com) (for upload).

It's better to self-host an instance. If you do so, please replace all `oranc.li7g.com` below with your instance.

1. Set your credentials.

   ```bash
   export AWS_ACCESS_KEY_ID=(echo -n "{USERNAME}:{PASSWORD}" | base64)
   export AWS_SECRET_ACCESS_KEY="_" # can be anything
   ```

2. Prepare your signing keys.

   ```console
   $ nix key generate-secret --key-name {KEY_NAME} > {PRIVATE_KEY_FILE}
   $ cat {PRIVATE_KEY_FILE} | nix key convert-secret-to-public
   {PUBLIC_KEY}
   ```

3. Build and sign something.

   ```bash
   nix build
   nix store sign ./result --recursive --key-file {PRIVATE_KEY_FILE}
   ```

4. Push to your OCI registry.

   ```bash
   nix copy ./result \
     --to "s3://{REPOSITORY_NAME_PART_2}?endpoint=https://oranc.li7g.com/{OCI_REGISTRY}/{REPOSITORY_NAME_PART_1}"
   ```

   Currently, only support repository names consist of two parts separated by `'/'`.

   For example, `s3://cache?endpoint=https://oranc.li7g.com/ghcr.io/linyinfeng` uploads to `ghcr.io/linyinfeng/cache`.

   Use <https://oranc-upload.li7g.com> if the size of `nar` files exceeded Cloudflare's limit.

5. Use your OCI registry as a cache.

   In `nix.conf`:

   ```text
   substituters = https://oranc.li7g.com/{OCI_REGISTRY}/{REPOSITORY_NAME_PART_1}/{REPOSITORY_NAME_PART_2}

   trusted-public-keys = {PUBLIC_KEY}
   ```

   Make sure your OCI repository allows anonymous pull.

   Otherwise, you need to use the following authorization header to access it through oranc.

   ```text
   AWS4-HMAC-SHA256 Credential={AWS_ACCESS_KEY_ID}/
   ```
