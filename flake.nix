{
  inputs.infra.url = "github:cubething-qproj/infra";
  outputs = { self, infra }: {
    devShells = infra.devShells;
  };
}
