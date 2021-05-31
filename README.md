# deploy-flake - an experimental tool for deploying a nix flake to a remote system

This tool is extremely inspired by (and in some ways, a reimagining of) [serokell/deploy-rs](//github.com/serokell/deploy-rs), which is a mature deploy tool that you should use if your remote systems matter to you.

Both of these tools are written in Rust, deal with nix flakes and let you deploy a system configuration defined in a nix flake to a nixos system that you access via ssh.

## Differences between `deploy-flake` and `deploy-rs`

deploy-flake has several **shortcomings and TODOs** for now:

* No rollbacks if applying a config fails.

* No checking whether the system configuration is reasonable (if you turn off your sshd, you'll be locked out).

* No timeouts (if system config applying hangs, it will never cancel & roll back).

I think there's a way to do all of these without sacrificing functionality. Only a matter of implementing them. (Contributions welcome! And I'll keep whittling away at this...)

What `deploy-flake` does that I think are advantages over `deploy-rs`:

* `deploy-flake` runs the build on the target system, eliminating the need for remote build servers if you are deploying nixos configs from a darwin system.

* Only this single binary that you have to build run on your local machine, no cross-compilation for other platforms needed (yet, but any binaries that run remotely will live in a different cargo project and be possible to pin separately).

* Better system activation story: The flake configuration is first applied via `nixos-rebuild test`, and only if that works, added to the boot entries via the equivalent of `nixos-rebuild boot`.

* Nicer story around running the test process in the background: It uses `systemd-run` to spawn the activation as a systemd unit, which will allow the control process to get disconnected at any point in time & the deployment can continue.

# Setting up

To run deploy-flake with your flake definition, add the following inputs into your flake.nix:

```nix
inputs = {
  # ...
  deploy-flake = {
    url = "github:antifuchs/deploy-flake";

    # The following are optional, but probably a good idea if you have these inputs:
    # inputs.nixpkgs.follows = "nixpkgs";
    # inputs.naersk.follows = "naersk";
  };
}
```

and the following to the outputs for your platform (you'll probably want to use [flake-utils](https://github.com/numtide/flake-utils) for those clauses):

```nix
outputs =
  { #...
  , deploy-flake
  }:
  # ...
  {
    apps.deploy-flake = deploy-flake.apps.deploy-flake.${system};
  }
```

# Usage

Once set up in your flake.nix, you can invoke `deploy-flake` like this:

```sh
$ nix run ./#deploy-flake -- destination-host
```

That will copy a snapshot of the flake onto the host `destination-host`, build & activate it and if that suceeds, set the configuration up to be booted.
