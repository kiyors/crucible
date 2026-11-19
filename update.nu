#!/usr/bin/env nix
#! nix shell --inputs-from . nixpkgs#nushell -c nu

const cargo_repo = "huacnlee/cargo"
const rust_dist = "https://static.rust-lang.org/dist"

# Nix system -> Rust host triple, for every system in lib/systems.nix.
const platforms = {
  "x86_64-linux": "x86_64-unknown-linux-gnu"
  "aarch64-linux": "aarch64-unknown-linux-gnu"
  "aarch64-darwin": "aarch64-apple-darwin"
}

# The pinned nightly is the newest one that ships all of these on every host,
# so `nightly` never lacks a tool. Cranelift is in the list because
# huacnlee/cargo builds local crates with it on a nightly rustc.
const nightly_required = [
  "rustc"
  "cargo"
  "rust-std"
  "rust-src"
  "clippy-preview"
  "rustfmt-preview"
  "llvm-tools-preview"
  "rust-analyzer-preview"
  "rustc-codegen-cranelift-preview"
]

# How far back to look for a complete nightly.
const nightly_max_days = 30

def root-dir []: nothing -> string {
  $env.FILE_PWD
}

def sources-path []: nothing -> string {
  root-dir | path join "sources.json"
}

# `save` writes exactly what it is given, so re-add the trailing newline every
# other tool in the repo expects of a text file
def save-json [path: path]: record -> nothing {
  to json --indent 2
  | $"($in)\n"
  | save --force $path
}

# The API allows 60 unauthenticated requests an hour; CI passes its token.
def github-headers []: nothing -> record {
  let token = $env.GITHUB_TOKEN? | default ""
  if ($token | is-empty) { {} } else { {Authorization: $"Bearer ($token)"} }
}

# GitHub's release CDN times out now and then; a scheduled job should not fail on that.
def http-get-retry [url: string, --headers: record = {}, attempts: int = 3]: nothing -> any {
  for attempt in 1..$attempts {
    try {
      return (http get --raw --max-time 60sec --headers $headers $url)
    } catch {|err|
      if $attempt == $attempts { error make {msg: $"GET ($url) failed: ($err.msg)"} }
      print $"  retrying ($url) after: ($err.msg)"
      sleep (2sec * $attempt)
    }
  }
}

# ---------------------------------------------------------------- huacnlee/cargo

def cargo-latest-version []: nothing -> string {
  http-get-retry --headers (github-headers) $"https://api.github.com/repos/($cargo_repo)/releases/latest"
  | from json
  | get tag_name
  | str replace --regex '^v' ''
}

def cargo-release-url [version: string]: nothing -> string {
  $"https://github.com/($cargo_repo)/releases/download/v($version)"
}

# Every release publishes SHA256SUMS, so no tarball has to be downloaded to
# learn its hash.
def cargo-platforms [version: string]: nothing -> record {
  let sums = http-get-retry $"(cargo-release-url $version)/SHA256SUMS"
    | lines
    | parse --regex '^(?<hex>[0-9a-f]{64})\s+\*?(?<file>\S+)$'

  $platforms
  | items {|nix_platform, triple|
    let file = $"cargo-($version)-($triple).tar.gz"
    let entry = $sums | where file == $file
    if ($entry | is-empty) {
      error make {msg: $"SHA256SUMS of v($version) has no ($file)"}
    }
    let hash = ^nix hash convert --hash-algo sha256 --to sri ($entry | first | get hex) | str trim
    print $"  ($nix_platform): ($hash)"
    {$nix_platform: {url: $"(cargo-release-url $version)/($file)", hash: $hash}}
  }
  | into record
}

# Update sources.json to the latest huacnlee/cargo release, or to `--version`.
def "main cargo" [--version: string] {
  let current = open (sources-path) | get version
  let version = $version | default (cargo-latest-version)
  print $"huacnlee/cargo: ($current) -> ($version)"

  {
    version: $version
    platforms: (cargo-platforms $version)
  }
  | save-json (sources-path)
}

# ---------------------------------------------------------------- Rust channels

def fetch-manifest [url: string]: nothing -> record {
  http-get-retry $url | from toml
}

# What a toolchain on one of our hosts can use: every package for a host
# triple, rust-src (target "*"), and rust-std for every target, for cross
# builds. Only xz tarballs, and only what is available.
def trim-manifest []: record -> record {
  let manifest = $in
  let hosts = $platforms | values

  let pkg = $manifest.pkg
    | items {|name, pkg|
      let targets = $pkg.target
        | items {|target, src|
          let wanted = $name == "rust-std" or $target == "*" or $target in $hosts
          if ($wanted and ($src.available? | default false)) {
            {$target: {url: ($src.xz_url? | default $src.url), hash: ($src.xz_hash? | default $src.hash)}}
          }
        }
        | compact

      if ($name not-in ["rust" "reproducible-artifacts"] and ($targets | is-not-empty)) {
        {$name: {version: $pkg.version, target: ($targets | into record)}}
      }
    }
    | compact
    | into record

  {
    date: $manifest.date
    version: ($manifest.pkg.rustc.version | split row " " | first)
    profiles: $manifest.profiles
    renames: $manifest.renames
    pkg: $pkg
  }
}

def missing-for-hosts [manifest: record, required: list<string>]: nothing -> list<string> {
  $required
  | each {|name|
    $platforms
    | values
    | where {|host|
      let target = $manifest.pkg | get -o $name | get -o target
      let src = if $target == null { null } else { $target | get -o $host | default ($target | get -o "*") }
      not ($src.available? | default false)
    }
    | each {|host| $"($name)/($host)" }
  }
  | flatten
}

# The newest nightly that has every required component on every host.
def complete-nightly []: nothing -> any {
  let latest = fetch-manifest $"($rust_dist)/channel-rust-nightly.toml"
  let start = $latest.date | into datetime

  for days in 0..$nightly_max_days {
    let date = $start - ($days * 1day) | format date "%Y-%m-%d"
    let manifest = if $days == 0 { $latest } else {
      try { fetch-manifest $"($rust_dist)/($date)/channel-rust-nightly.toml" } catch { null }
    }
    if $manifest == null { continue }

    let missing = missing-for-hosts $manifest $nightly_required
    if ($missing | is-empty) {
      print $"  nightly: ($date) is complete"
      return $manifest
    }
    print $"  nightly: ($date) lacks ($missing | str join ', ')"
  }
  error make {msg: $"no complete nightly in the last ($nightly_max_days) days"}
}

# Update data/{stable,beta,nightly}.json from static.rust-lang.org.
def "main rust" [] {
  let data_dir = root-dir | path join "data"
  mkdir $data_dir

  for channel in ["stable" "beta"] {
    let trimmed = fetch-manifest $"($rust_dist)/channel-rust-($channel).toml" | trim-manifest
    print $"  ($channel): ($trimmed.version) \(($trimmed.date)\)"
    $trimmed | save-json ($data_dir | path join $"($channel).json")
  }

  let nightly = complete-nightly | trim-manifest
  print $"  nightly: ($nightly.version) \(($nightly.date)\)"
  $nightly | save-json ($data_dir | path join "nightly.json")
}

# Update everything: huacnlee/cargo and the pinned Rust channels.
def main [] {
  main cargo
  main rust
}
