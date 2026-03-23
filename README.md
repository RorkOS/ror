# ror

The package manager for RorkOS. Installs, removes, and updates binary packages from git-based repositories.

```sh
      --search <QUERY>   #search                 
  -s, --sync             #synchronizes the repository         
  -d, --delete <PACKAGE> #removing a package              
  -l, --list-installed   #installed packages  
      --info <PACKAGE>   #package information
  -v, --version          #package manager version
  -g, --gen-config       #generating configuration for the package manager
  -i, --install <PACKAGE_OR_GROUP>...  #example: ror -i @base/bash
      --update <PACKAGE> #update one package
      --dry-run          #simulates the execution of a command
  -u, --upgrade          #updates the entire system
      --repo-add <NAME> <URL>#adding a repository
      --repo-remove <NAME>   #deletes the repository
      --repo-list         #list of repositories
      --build-rootfs      #creates rootfs             
      --rootfs-group <GROUP> #creates rootfs with group          
      --rootfs-target <TARGET>#installs root force in a separate folder         
      --rootfs-arch <ROOTFS_ARCH>      [default: native] #installs rootfs for a specific architecture
  -h, --help                           Print help
```


Config is located at `/var/ror/ror.conf`:

```ini
[global]
ignore_speed = false
strict_gpg = false

[repositories.rorkos]
url = "https:
mirror = "https:
```



---

## Package Format

Packages are YAML files stored in the repository under `<category>/<n>/<n>.yaml`:

```yaml
name: vim
version: "9.1.0"
description: "Vi IMproved - enhanced vi editor"
license: "Vim"
homepage: "https://www.vim.org"

provides:
  - editor

depends:
  - "libncurses"
  - ["glibc", "musl"] #choice of dependency

conflicts:
  - vim-tiny #packet conflict

binaries:
  - arch: amd64
    type: tar.gz
    filename: "vim-9.1.0-pkg-amd64-rorkos.tar.gz"
    sha256: ""
    mirrors:
      - "https://repo.example.com/packages/{filename}"

install_steps: |
  update-alternatives --install /usr/bin/vi vi /usr/bin/vim 100

delete_steps: |
  update-alternatives --remove vi /usr/bin/vim
```

---

Group files live at `/var/ror/packages/groups/<n>.yaml`:

```yaml
name: base
description: "Minimal RorkOS base system"
packages:
  - "musl"
  - "busybox"
  - "runit"
  - "ror"
```

---

## Building a Rootfs

```sh
ror --build-rootfs \
    --rootfs-group base \
    --rootfs-target /mnt/rootfs \
    --rootfs-arch amd64
```

Installs all packages from the group into an empty directory, then runs each package's `install_steps` inside a chroot.

---

## Debug

```sh
ROR_DEBUG=1 ror -i curl
```

---

## File Structure

```
/etc/ror/
├── ror.conf            # config
├── installed.json      # installed package database
/var/ror/packages/
    ├── groups/         # group definitions
    └── <category>/
        └── <pkg>/
            └── <pkg>.yaml
```
