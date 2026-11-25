# The flake's starter projects, exposed as `templates.<name>` for
# `nix flake init -t github:kiyors/crucible#<name>`. Each path is a template
# directory with its own flake.nix; `c` and `cpp` are aliases of `c-cpp`, and
# `hash` points at the `hashi` directory.
{
  default = {
    path = ./templates/default;
    description = "Rust project pinned from rust-toolchain.toml, with a buildRustPackage package (the default template)";
  };

  node = {
    path = ./templates/node;
    description = "Node.js with pnpm pinned from crucible and run by that Node.js";
  };

  python = {
    path = ./templates/python;
    description = "Python with CPython and uv pinned from crucible";
  };

  go = {
    path = ./templates/go;
    description = "Go with the newest supported release pinned from crucible";
  };

  zig = {
    path = ./templates/zig;
    description = "Zig with the newest stable release pinned from crucible";
  };

  deno = {
    path = ./templates/deno;
    description = "Deno 2 pinned from crucible";
  };

  bun = {
    path = ./templates/bun;
    description = "Bun pinned from crucible, without Node.js";
  };

  c-cpp = {
    path = ./templates/c-cpp;
    description = "C/C++ tools from nixpkgs: clang, CMake, Conan, vcpkg, linters and gdb on Linux";
  };

  elixir = {
    path = ./templates/elixir;
    description = "Elixir 1.17 and Erlang/OTP 27 from nixpkgs, with Hex, mix and rebar3";
  };

  elm = {
    path = ./templates/elm;
    description = "Elm pinned from elm.json by crucible, with elm2nix from nixpkgs";
  };

  empty = {
    path = ./templates/empty;
    description = "Empty nixpkgs dev shell to adapt to your own environment";
  };

  hash = {
    path = ./templates/hashi;
    description = "HashiCorp tools from nixpkgs: Terraform, Packer, Nomad, Vault and Terragrunt (allowUnfree)";
  };

  java = {
    path = ./templates/java;
    description = "Java 25 (JDK, Gradle, Maven, Lombok) from nixpkgs";
  };

  jupyter = {
    path = ./templates/jupyter;
    description = "Jupyter from nixpkgs with Python 3.11, Poetry, ipykernel and a .venv";
  };

  kotlin = {
    path = ./templates/kotlin;
    description = "Kotlin on Java 25 from nixpkgs, with Gradle";
  };

  laravel = {
    path = ./templates/laravel;
    description = "PHP, Composer and Laravel from nixpkgs";
  };

  nim = {
    path = ./templates/nim;
    description = "Nim and Nimble from nixpkgs";
  };

  nix = {
    path = ./templates/nix;
    description = "Nix tooling from nixpkgs: nixd, nixfmt, statix, cachix and vulnix";
  };

  odin = {
    path = ./templates/odin;
    description = "Odin from nixpkgs";
  };

  php = {
    path = ./templates/php;
    description = "PHP and Composer from nixpkgs";
  };

  rust = {
    path = ./templates/rust;
    description = "Rust toolchain pinned from crucible, with native build inputs";
  };

  scala = {
    path = ./templates/scala;
    description = "Scala 3 and sbt on Java 25 from nixpkgs";
  };

  shell = {
    path = ./templates/shell;
    description = "Shell development from nixpkgs with shellcheck";
  };

  swift = {
    path = ./templates/swift;
    description = "Swift and SourceKit-LSP from nixpkgs";
  };

  uv = {
    path = ./templates/uv;
    description = "Python (CPython + uv) and Node.js, both pinned from crucible, with a uv venv";
  };

  c = {
    path = ./templates/c-cpp;
    description = "Alias for the c-cpp C/C++ nixpkgs template";
  };

  cpp = {
    path = ./templates/c-cpp;
    description = "Alias for the c-cpp C/C++ nixpkgs template";
  };
}
