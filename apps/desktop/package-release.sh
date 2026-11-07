#!/usr/bin/env bash
# Build redistributable packages for a Stride POS desktop release.
#
# Usage: package-release.sh <binary> <version> <outdir> <target>
#
#   binary   path to the built binary (desktop or desktop.exe)
#   version  release version, e.g. 0.1.0
#   outdir   directory to write packages into
#   target   rust target triple, e.g. x86_64-unknown-linux-gnu
#
set -euo pipefail

binary="$1"
version="$2"
outdir="$3"
target="$4"

name="stride-pos"
base="${name}-${version}-${target}"
mkdir -p "$outdir"

if [[ "$target" == *-pc-windows-msvc ]]; then
    # Windows: ship the executable and README in a zip.
    root="dist/${base}"
    rm -rf "$root"
    mkdir -p "$root"
    cp "$binary" "$root/stride-pos.exe"
    cp README.md "$root/README.md"
    if command -v zip >/dev/null 2>&1; then
        (cd dist && zip -r "${base}.zip" "$base")
    else
        powershell.exe -NoProfile -Command \
            "Compress-Archive -Path 'dist\\${base}' -DestinationPath 'dist\\${base}.zip' -Force"
    fi
    rm -rf "$root"
    exit 0
fi

# Linux: tar.gz, .deb (Debian/Ubuntu) and .rpm (Fedora/RHEL).
root="dist/${base}"
rm -rf "$root"
mkdir -p "$root/usr/bin" "$root/usr/share/doc/${name}"
install -m 0755 "$binary" "$root/usr/bin/${name}"
cp README.md "$root/usr/share/doc/${name}/README.md"

# Source tarball.
(cd dist && tar -czf "${base}.tar.gz" "$base")
rm -rf "$root"

# Debian package.
debroot="dist/${base}-deb"
rm -rf "$debroot"
mkdir -p "$debroot/DEBIAN" "$debroot/usr/bin" "$debroot/usr/share/doc/${name}"
cat > "$debroot/DEBIAN/control" <<EOF
Package: ${name}
Version: ${version}
Section: utils
Priority: optional
Architecture: amd64
Maintainer: Stride POS <packaging@stride.example>
Description: Stride point-of-sale desktop till
 Stride POS is a local-first point-of-sale till for retail shops.
EOF
install -m 0755 "$binary" "$debroot/usr/bin/${name}"
cp README.md "$debroot/usr/share/doc/${name}/README.md"
dpkg-deb --build "$debroot" "dist/${base}.deb" >/dev/null
rm -rf "$debroot"

# Fedora / Red Hat package.
if command -v rpmbuild >/dev/null 2>&1; then
    topdir="dist/rpmbuild-${target}"
    rm -rf "$topdir"
    mkdir -p "$topdir"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
    cp "$binary" "$topdir/SOURCES/${name}"
    cat > "$topdir/SPECS/${name}.spec" <<EOF
Name:           ${name}
Version:        ${version}
Release:        1%{?dist}
Summary:        Stride point-of-sale desktop till
License:        Proprietary
BuildArch:      x86_64
Source0:        ${name}

%description
Stride POS is a local-first point-of-sale till for retail shops.

%install
mkdir -p %{buildroot}/usr/bin
install -m 0755 %{SOURCE0} %{buildroot}/usr/bin/${name}

%files
/usr/bin/${name}

%changelog
* $(date '+%a %b %d %Y') Stride POS <packaging@stride.example> - ${version}-1
- Initial release
EOF
    rpmbuild -bb --define "_topdir $topdir" "$topdir/SPECS/${name}.spec" >/dev/null
    cp "$topdir/RPMS/x86_64"/*.rpm "dist/${base}.rpm"
    rm -rf "$topdir"
fi
