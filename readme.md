Git Cleaner
---

I'm lazy and never clean up my remotes, so I built a tool to do so.

Requirements;
Cargo/Rust toolchain
Git cli for remote delete interop

usage;
clone this repo; cd into it; cargo install --path .

then in the desired repo run `git-cleaner --author "me@work.com" --before "90d"`

it is recommended to run with `--dry` before just to make sure its relatively correct
i also recommend, if you want to go nuclear, to run with `--dry` first, then run it again but replace `--dry` with `--yes`
