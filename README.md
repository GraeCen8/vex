# Vex

Vex is a tiny, educational, git-like implementation written in Rust. It supports a handful of core commands and a simplified index format so you can explore how git plumbing fits together without the full surface area of git itself.

## Notes
- This is not a drop-in replacement for git.
- The index format is simplified and not compatible with git’s index file.

## Commands
- `init [path]`
- `hash-object [-w] <path>`
- `cat-file <oid>`
- `ls-tree <tree-ish>`
- `add <paths...>`
- `ls-files`
- `status`
- `rm [--cached] <paths...>`
- `checkout <tree-ish>`
- `commit -m <message>`
- `log [rev]`
- `rev-parse <name>`
- `show-ref [--head]`
- `tag <name> [target]`
- `check-ignore <paths...>`

## Examples
```bash
cargo run -- init
echo "hello" > hello.txt
cargo run -- add hello.txt
cargo run -- commit -m "first commit"
cargo run -- log
```
