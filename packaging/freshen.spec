# RPM spec file for freshen.
#
# Builds the freshen binary ahead of time with cargo and packages it as an
# .rpm that installs freshen to %{_bindir}/freshen (mode 0755), mirroring the
# .deb package (ci-proposals/release-deb.yml) which installs to /usr/bin/freshen.
#
# The CI job stages the prebuilt binary into %{_sourcedir} as `freshen` and
# runs `rpmbuild -bb packaging/freshen.spec`, overriding %{_arch} so both
# x86_64 and aarch64 RPMs can be produced from an x86_64 runner.

Name:           freshen
Version:        0.1.0
Release:        1%{?dist}
Summary:        One command to keep a developer machine current

License:        MIT
URL:            https://github.com/yildirimarda/freshen
Source0:        freshen

%description
Keep a developer machine up to date. freshen updates Homebrew, apt, dnf,
winget, Chocolatey, Scoop, uv, mise, Docker, Colima and more from a single
command.

%prep
# The prebuilt freshen binary is staged into %{_sourcedir} by the CI job; no
# source archive to unpack or patch.

%build
# freshen is compiled ahead of time by cargo; nothing to build here.

%install
rm -rf "$RPM_BUILD_ROOT"
mkdir -p %{buildroot}%{_bindir}
install -m 0755 %{SOURCE0} %{buildroot}%{_bindir}/freshen

%files
%{_bindir}/freshen
