"""AccountsService mock for the helper's tests (python-dbusmock has no
accountsservice template).

It claims org.freedesktop.Accounts on the (private) system bus and does what
AccountsService does to the files of a throwaway root: CreateUser appends to
etc/passwd, etc/shadow (locked) and etc/group (wheel for an administrator),
makes the home from etc/skel and owns it; the user object's SetPassword writes
the hash into etc/shadow; DeleteUser removes them.

Parameters (JSON, -p): root (the throwaway root), first_uid.

Control interface org.freedesktop.DBus.Mock: Fail(method) makes the next call
of that method fail once, GetLog() returns what was called (without secrets:
SetPassword logs only the user path), SetDelay(seconds) delays CreateUser.
"""

import os
import shutil
import time

import dbus
import dbus.service
from dbusmock import MOCK_IFACE, mockobject

BUS_NAME = "org.freedesktop.Accounts"
MAIN_OBJ = "/org/freedesktop/Accounts"
MAIN_IFACE = "org.freedesktop.Accounts"
USER_IFACE = "org.freedesktop.Accounts.User"
SYSTEM_BUS = True


def load(mock, parameters):
    mock.root = parameters["root"]
    mock.next_uid = int(parameters["first_uid"])
    mock.wlog = []
    mock.failing = set()
    mock.delay = 0.0


def _fail(mock, method):
    if method in mock.failing:
        mock.failing.discard(method)
        raise dbus.exceptions.DBusException(
            "injected failure in " + method, name="org.freedesktop.Accounts.Error.Failed"
        )


def _lines(path):
    try:
        with open(path) as f:
            return f.read().splitlines()
    except FileNotFoundError:
        return []


def _write(path, lines):
    with open(path, "w") as f:
        f.write("\n".join(lines) + ("\n" if lines else ""))


class User(mockobject.DBusMockObject):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.main = kwargs["mock_data"]["main"]
        self.user = kwargs["mock_data"]["name"]

    @dbus.service.method(USER_IFACE, in_signature="ss", out_signature="")
    def SetPassword(self, password, hint):
        main = self.main
        main.wlog.append("SetPassword " + self.path)
        _fail(main, "SetPassword")
        path = os.path.join(main.root, "etc/shadow")
        prefix = self.user + ":"
        lines = [line for line in _lines(path) if not line.startswith(prefix)]
        lines.append("%s:%s:19000::::::" % (self.user, password))
        _write(path, lines)


@dbus.service.method(MAIN_IFACE, in_signature="ssi", out_signature="o")
def CreateUser(self, name, fullname, account_type):
    self.wlog.append("CreateUser %s %d" % (name, account_type))
    _fail(self, "CreateUser")
    time.sleep(self.delay)
    root = self.root
    uid = self.next_uid
    self.next_uid += 1
    passwd = os.path.join(root, "etc/passwd")
    for line in _lines(passwd):
        if line.split(":")[0] == name:
            raise dbus.exceptions.DBusException(
                "user exists", name="org.freedesktop.Accounts.Error.Failed"
            )
    _write(passwd, _lines(passwd) + ["%s:x:%d:%d:%s:/home/%s:/bin/bash" % (name, uid, uid, fullname, name)])
    shadow = os.path.join(root, "etc/shadow")
    _write(shadow, _lines(shadow) + ["%s:!:19000::::::" % name])
    group = os.path.join(root, "etc/group")
    out = []
    for line in _lines(group):
        if line.startswith("wheel:") and account_type == 1:
            f = line.split(":")
            members = [m for m in f[3].split(",") if m] + [name]
            line = ":".join(f[:3] + [",".join(members)])
        out.append(line)
    out.append("%s:x:%d:" % (name, uid))
    _write(group, out)
    home = os.path.join(root, "home", name)
    os.makedirs(home, exist_ok=True)
    skel = os.path.join(root, "etc/skel")
    for entry in os.listdir(skel) if os.path.isdir(skel) else []:
        src = os.path.join(skel, entry)
        if os.path.isdir(src):
            shutil.copytree(src, os.path.join(home, entry), dirs_exist_ok=True)
        else:
            shutil.copy(src, os.path.join(home, entry))
    for dirpath, dirnames, filenames in os.walk(home):
        for n in [dirpath] + [os.path.join(dirpath, x) for x in filenames]:
            os.chown(n, uid, uid)
    os.chmod(home, 0o755)
    path = "/org/freedesktop/Accounts/User%d" % uid
    self.AddObject(path, USER_IFACE, {}, [], mock_class=User, mock_data={"main": self, "name": name})
    return dbus.ObjectPath(path)


@dbus.service.method(MAIN_IFACE, in_signature="xb", out_signature="")
def DeleteUser(self, uid, remove_files):
    self.wlog.append("DeleteUser %d %s" % (uid, "true" if remove_files else "false"))
    _fail(self, "DeleteUser")
    root = self.root
    passwd = os.path.join(root, "etc/passwd")
    name = None
    for line in _lines(passwd):
        f = line.split(":")
        if f[2] == str(uid):
            name = f[0]
    if name is None:
        raise dbus.exceptions.DBusException("no such user", name="org.freedesktop.Accounts.Error.Failed")
    for rel in ("etc/passwd", "etc/shadow"):
        p = os.path.join(root, rel)
        _write(p, [line for line in _lines(p) if not line.startswith(name + ":")])
    group = os.path.join(root, "etc/group")
    out = []
    for line in _lines(group):
        f = line.split(":")
        if f[0] == name:
            continue
        if len(f) > 3:
            f[3] = ",".join(m for m in f[3].split(",") if m and m != name)
        out.append(":".join(f))
    _write(group, out)
    if remove_files:
        shutil.rmtree(os.path.join(root, "home", name), ignore_errors=True)
    obj = "/org/freedesktop/Accounts/User%d" % uid
    if obj in mockobject.objects:
        self.RemoveObject(obj)


@dbus.service.method(MOCK_IFACE, in_signature="s", out_signature="")
def Fail(self, method):
    self.failing.add(method)


@dbus.service.method(MOCK_IFACE, in_signature="", out_signature="as")
def GetLog(self):
    return dbus.Array(self.wlog, signature="s")


@dbus.service.method(MOCK_IFACE, in_signature="d", out_signature="")
def SetDelay(self, seconds):
    self.delay = float(seconds)
