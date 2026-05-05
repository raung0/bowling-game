# Workflow

Cargo watch is recommended to have installed, generally to work on the project
you would have 2 editors open at a time:

- The text editor
- Godot itself

Then have a couple terminals opened to constantly recompile the rust code:

```sh
# Godot extension
cargo watch -x 'build -p gdext'
# Web server
cargo watch -x 'run -p server'
```

These watch for file changes, and when gdext is recompiled, Godot will
hot-reload the library file.

# Development environment

A Nix development shell is provided in [flake.nix](./flake.nix). To enter it,
use `nix develop`. If you have direnv-nix installed, you can just run `direnv
allow` once and when you cd into the directory you will have the environment
ready for you.

The Nix development environment has Godot in it. To use it, simply start it
from the development shell (`godot4`) and open up the "godot" directory.

If you don't have Nix installed yet run the following command:

```sh
curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
```

This will install Nix for you (on Linux and macOS), enabling flakes as well.

For development under Windows, you will have to install Rust and Godot 4
manually, or install Nix under WSL. Your choice.

If you don't wish to use Nix, you are on your own. Sorry not sorry!

