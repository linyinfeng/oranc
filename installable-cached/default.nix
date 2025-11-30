{
  stdenv,
  lib,
  gnutar,
  formats,

  self,
}:
let
  json = formats.json { };
  cachedJson = json.generate "cached.json" (
    lib.mapAttrs (_system: ps: ps.oranc) self.packages
    // {
      default = "github:linyinfeng/oranc/${self.rev or "main"}#oranc";
    }
  );
in
cachedJson
