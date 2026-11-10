#!/bin/sh
set -e
install -m 644 "$(dirname "$(readlink -f "$0")")/70-soundhold.rules" /etc/udev/rules.d/70-soundhold.rules
udevadm control --reload
udevadm trigger --subsystem-match=hidraw
