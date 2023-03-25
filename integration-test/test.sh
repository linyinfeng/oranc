#!@shell@

set -e

function info {
  echo -en "\e[32m" # Green
  echo -n "$@"
  echo -e "\e[0m"
}

function stage {
  echo
  info "--" "$@"
  echo
}

info "arguments:"
info "  PACKAGE_FOR_TEST: '$PACKAGE_FOR_TEST'"
info "  PATH: '$PATH'"
info "  RUST_LOG: '$RUST_LOG'"

stage "setting up working directory..."

dir="/tmp/oranc-integration-test"
mkdir -p "$dir"
cd "$dir"
echo "working directory: '$PWD'"

stage "setting up nix..."

cat >nix.conf <<EOF
experimental-features = nix-command flakes auto-allocate-uids
auto-allocate-uids = true
EOF
mkdir -p /etc/nix
echo -en "\ninclude $dir/nix.conf\n" >>/etc/nix/nix.conf

groupadd nixbld --system

stage "show nix.conf..."

nix show-config

stage "generate temporary key pair"

nix key generate-secret --key-name "oranc-test" >secret
cat secret | nix key convert-secret-to-public >public
echo "secret key for test: $(cat secret)"
echo "public key for test: $(cat public)"

stage "setting up variables..."

registry="registry:5000"
repository="test-user/oranc-cache"
substituter="http://oranc/registry:5000/$repository"
public_key="$(cat public)"
export ORANC_SIGNING_KEY="$(cat secret)"

stage "intialize registry"

oranc push --no-ssl --registry "$registry" --repository "$repository" initialize
curl "$substituter/nix-cache-info" -v

stage "get test packages"

nix path-info --derivation --recursive "$PACKAGE_FOR_TEST" >derivers
cat derivers | xargs nix build --no-link --print-out-paths >derived
cat derived | xargs nix path-info --recursive >derived_closure
cat derivers derived_closure >store_paths

echo "derivers: $(cat derivers | wc -l)"
echo "derived: $(cat derived | wc -l)"
echo "derived_closure: $(cat derived_closure | wc -l)"
echo "store_paths: $(cat store_paths | wc -l)"

stage "push to packages"

echo "store paths count: $(cat store_paths | wc -l)"
cat store_paths

# sign first, then push with --already-signed
# oranc will check its signature matches already exists signature
cat store_paths | xargs nix store sign --key-file secret --verbose
cat store_paths |
  oranc push \
    --no-ssl \
    --registry "$registry" \
    --repository "$repository" \
    --excluded-signing-key-pattern '^$' \
    --already-signed

stage "verify remote store"

cat store_paths | xargs nix store verify \
  --store "$substituter" \
  --trusted-public-keys "$public_key" \
  --verbose

stage "gc local store"

nix store gc

stage "get test package from registry"

# instantiate derivations again
nix path-info --derivation --recursive "$PACKAGE_FOR_TEST" >/dev/null
cat derived_closure |
  xargs nix build \
    --no-link \
    --max-jobs 0 \
    --substituters "$substituter" \
    --trusted-public-keys "$public_key" \
    --verbose

stage "verify local store"

cat store_paths | xargs nix store verify \
  --trusted-public-keys "$public_key" \
  --verbose
