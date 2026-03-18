# yggterm npm launcher

This package provides the `yggterm` command for `npx` and global npm installs.

It downloads the matching GitHub release binary for the current package version on first run and then executes it locally.

## Usage

```bash
npx -y yggterm
```

or:

```bash
npm install -g yggterm
yggterm
```

## Notes

- Current binary support in this launcher is `linux-x86_64`.
- The binary is fetched from `https://github.com/yggdrasilhq/yggterm/releases`.
- Set `YGGTERM_REPO` to point at a different GitHub repository slug if needed.
