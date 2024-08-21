## buildfs

`buildfs` is an easy-to-use CLI tool meant for both manual and automated (CI) use to create raw filesystem (**rootfs**) images that can be used to boot virtual machines.
It has 4 subcommands: `pack` and `unpack` to manage packages, `dry-run` to test the correctness of a package and `run` to actually fully execute the package. A package can be a single TOML build script, a directory with the build script and references to other resources, or such a directory compressed in either a `.tar` or a `.tar.gz`.

### Getting started

1. `cargo install buildfs`.
2. Root privileges are needed (for `mkfs` and `mount/umount`), so ensure you can run `sudo` on the target machine and that it is running a Linux distribution.
3. Insert the following build script contents into `/tmp/build_script.toml`. This is a simple configuration that will make a minified bootable Debian root filesystem from the `docker.io/library/debian:bookworm-slim` image:
```toml
[filesystem]
type = "Ext4"
size_mib = 250

[container]
engine = "Docker"
rootful = true
wait_timeout_s = 1
image = { name = "docker.io/library/debian", tag = "bookworm-slim" }

[[commands]]
script_inline = """
#!/bin/bash

apt update
apt install -y udev systemd-sysv iputils-ping curl

rm -f /etc/systemd/system/multi-user.target.wants/systemd-resolved.service
rm -f /etc/systemd/system/dbus-org.freedesktop.resolve1.service
rm -f /etc/systemd/system/sysinit.target.wants/systemd-timesyncd.service

systemctl disable e2scrub_reap.service
rm -vf /etc/systemd/system/timers.target.wants/*

for console in ttyS0; do
    mkdir "/etc/systemd/system/serial-getty@$console.service.d/"
    cat <<'EOF' > "/etc/systemd/system/serial-getty@$console.service.d/override.conf"
[Service]
# systemd requires this empty ExecStart line to override
ExecStart=
ExecStart=-/sbin/agetty --autologin root -o '-p -- \\u' --keep-baud 115200,38400,9600 %I dumb
EOF
done

passwd -d root

rm -rf /usr/share/{doc,man,info,locale}

cat >> /etc/sysctl.conf <<EOF
# This avoids a SPECTRE vuln
kernel.unprivileged_bpf_disabled=1
EOF
"""

[[overlays]]
source_inline = """
nameserver 8.8.8.8
nameserver 8.8.4.4
nameserver 1.1.1.1
"""
destination = "/etc/resolv.conf"

[export.directories]
include = [ "/bin", "/etc", "/home", "/lib", "/lib64", "/root", "/sbin", "/usr" ]
create = [ "/var/lib/dpkg", "/dev", "/proc", "/sys", "/run", "/tmp", "/var/lib/systemd" ]
```
4. Ensure `~/.cargo/bin` is on your PATH so that `buildfs` is accessible and ensure Docker is installed (Podman is also supported, just change the value of `engine` in the build script and ensure a Podman Unix socket is bound).
5. Run `sudo buildfs run -o debian.ext4 /tmp/build_script.toml` and wait until it produces you a ready-to-use `debian.ext4` root filesystem!
