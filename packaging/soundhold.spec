%global debug_package %{nil}

Name:           soundhold
Version:        %{app_version}
Release:        1%{?dist}
Summary:        Head tracked spatial sound for Bluetooth headphones
License:        GPL-3.0-only AND CC-BY-SA-3.0
URL:            https://github.com/049er/soundhold
Requires:       pipewire
Requires:       pipewire-utils
Requires:       pipewire-module-filter-chain-sofa
Requires:       wireplumber
Requires:       bluez
Requires:       systemd
Recommends:     soundhold-window

%description
Keeps sound in place in the room as you turn your head.

%package window
Summary:        Window for Soundhold
Requires:       soundhold = %{version}-%{release}

%description window
Window for Soundhold.

%install
cd %{source_root}
install -Dm755 target/release/soundhold %{buildroot}%{_bindir}/soundhold
install -Dm755 target/release/soundhold-window %{buildroot}%{_bindir}/soundhold-window
install -Dm644 -t %{buildroot}%{_datadir}/soundhold sofa/*.sofa
install -Dm644 70-soundhold.rules %{buildroot}/usr/lib/udev/rules.d/70-soundhold.rules
install -Dm644 data/icons/hicolor/scalable/apps/soundhold.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/soundhold.svg
install -Dm644 data/icons/hicolor/symbolic/apps/soundhold-symbolic.svg %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/soundhold-symbolic.svg
install -Dm644 packaging/io.github.soundhold.Window.desktop %{buildroot}%{_datadir}/applications/io.github.soundhold.Window.desktop
install -Dm644 LICENSE %{buildroot}%{_datadir}/licenses/soundhold/LICENSE
install -Dm644 packaging/copyright %{buildroot}%{_datadir}/licenses/soundhold/copyright

%post
udevadm control --reload || :
udevadm trigger --subsystem-match=hidraw || :

%postun
udevadm control --reload || :

%files
%{_bindir}/soundhold
%{_datadir}/soundhold
/usr/lib/udev/rules.d/70-soundhold.rules
%{_datadir}/icons/hicolor/scalable/apps/soundhold.svg
%{_datadir}/icons/hicolor/symbolic/apps/soundhold-symbolic.svg
%{_datadir}/licenses/soundhold

%files window
%{_bindir}/soundhold-window
%{_datadir}/applications/io.github.soundhold.Window.desktop
