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

* Optional system closure self-check script: If you use `system.extraSystemBuilderCmds` to write a self-test program into your system closure, `deploy-flake` can optionally invoke it via the `--pre-activate-script=relative-pathname` option and will not kick off a deploy on the machine if that program returns a non-0 status code.

* Nicer story around running the test process in the background: It uses `systemd-run` to spawn the activation as a systemd unit, which will allow the control process to get disconnected at any point in time & the deployment can continue.

* Parallelism: You can deploy one flake to multiple hosts in one invocation, in parallel.

# Setting up

To run deploy-flake with your flake definition, add the following inputs into your flake.nix:

```nix
inputs = {
  # ...
  deploy-flake = {
    url = "github:boinkor-net/deploy-flake";

    # The following are optional, but probably a good idea if you have these inputs:
    # inputs.nixpkgs.follows = "nixpkgs";
    # inputs.rust-overlay.follows = "rust-overlay";
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
$ nix run ./#deploy-flake -- destination-host1 nixos://destination-host2/webserver
```

That will copy a snapshot of the flake onto the hosts `destination-host1` and `destination-host2`, build & activate it and if that suceeds, set the configuration up to be booted - all in parallel.

## Dealing with failure

There are a few reasons a deploy might fail: I'm going to talk about the two most common/important ones.

### `System is not healthy` before the deploy starts

`deploy-flake` expects that the running system is in a `running` state (as indicated by `systemctl status`) before it starts applying the system configuration change. This is meant to protect you from the case where deploying to a slightly-broken system causes even more damage by attempting to start or restart units that were working before but fail to come up in the degraded system.

When `deploy-flake` aborts with the message `System is not healthy.`, no changes ot the running system have occurred yet. You'll see a list of units that are currently in error states (and you can retrieve that same list by running `systemctl list-units --failed` on the remote system). Do whatever you need to do to get the units working again (restart them, stop them, use `systemctl reset-failed` or reboot the system), and then retry the deploy.

### Failure to apply the new system configuration

The more dangerous/annoying kind of failure occurs in the step that changes the running system (aka the `nixos-rebuild test` step): Units might fail to restart for whatever reason, and when they do, that could lock you out of the target system (e.g., if ssh or the network should fail to come back).

The semi-good news even when you're locked out is that your "boot" system configuration hasn't changed, so if you reboot the target system, it will come up in a configuration that has (hopefully!) worked previously.

In the less-terrible case, you aren't locked out but some unit failed to come up: You can get a list of those broken units and handle them accordingly (look at logs, restart them, etc).

In order to continue and get a deployable system back, the list of failed units must be empty (see the previous section). Once that's the case you can retry the deploy, and hopefully the "test" step will succeed.

Only when the "test" step succeeds does `deploy-flake` modify the boot configuration. Best of luck!
