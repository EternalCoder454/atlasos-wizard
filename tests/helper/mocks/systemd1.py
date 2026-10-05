"""systemd Manager mock for the helper's tests: RestartUnit and StartUnit log
the call (python-dbusmock's own systemd template has no RestartUnit).

Control interface org.freedesktop.DBus.Mock: GetLog() returns the calls.
"""

import dbus
import dbus.service
from dbusmock import MOCK_IFACE

BUS_NAME = "org.freedesktop.systemd1"
MAIN_OBJ = "/org/freedesktop/systemd1"
MAIN_IFACE = "org.freedesktop.systemd1.Manager"
SYSTEM_BUS = True


def load(mock, parameters):
    mock.wlog = []


@dbus.service.method(MAIN_IFACE, in_signature="ss", out_signature="o")
def RestartUnit(self, name, mode):
    self.wlog.append("RestartUnit %s %s" % (name, mode))
    return dbus.ObjectPath("/org/freedesktop/systemd1/job/1")


@dbus.service.method(MAIN_IFACE, in_signature="ss", out_signature="o")
def StartUnit(self, name, mode):
    self.wlog.append("StartUnit %s %s" % (name, mode))
    return dbus.ObjectPath("/org/freedesktop/systemd1/job/2")


@dbus.service.method(MOCK_IFACE, in_signature="", out_signature="as")
def GetLog(self):
    return dbus.Array(self.wlog, signature="s")
