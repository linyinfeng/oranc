# flake.nix contains many inputs
# provide a lightweight default.nix for faster installation
let
  inherit (builtins) fromJSON readFile fetchTree;
  flakeLock = fromJSON (readFile ./flake.lock);
  lockedNixpkgs = flakeLock.nodes.nixpkgs.locked;
  pinnedNixpkgs = fetchTree lockedNixpkgs;
in
{
  config ? { },
  pkgs ? import pinnedNixpkgs config,
}:
pkgs.callPackage ./package.nix { }
