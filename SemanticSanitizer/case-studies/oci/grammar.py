ctx.rule("START", "{OCI_CONFIG}")

# =============================================================================
# TOP-LEVEL CONFIG
# =============================================================================

ctx.script(
    "OCI_CONFIG",
    ["OCI_VERSION", "PLATFORM", "PROCESS", "ROOT", "LINUX", "MOUNTS", "ANNOTATIONS"],
    lambda version, platform, process, root, linux, mounts, annotations: b"{%s,%s,%s,%s,%s,%s,%s}"
    % (version, platform, process, root, linux, mounts, annotations),
)

ctx.script(
    "OCI_CONFIG",
    ["OCI_VERSION", "PLATFORM", "PROCESS", "ROOT", "LINUX", "MOUNTS", "HOOKS", "ANNOTATIONS"],
    lambda version, platform, process, root, linux, mounts, hooks, annotations: b"{%s,%s,%s,%s,%s,%s,%s,%s}"
    % (version, platform, process, root, linux, mounts, hooks, annotations),
)

ctx.script(
    "OCI_CONFIG",
    ["OCI_VERSION", "PLATFORM", "PROCESS", "ROOT", "LINUX", "MOUNTS", "HOSTNAME_FIELD", "ANNOTATIONS"],
    lambda version, platform, process, root, linux, mounts, hostname, annotations: b"{%s,%s,%s,%s,%s,%s,%s,%s}"
    % (version, platform, process, root, linux, mounts, hostname, annotations),
)

ctx.script(
    "OCI_CONFIG",
    ["OCI_VERSION", "PLATFORM", "PROCESS", "ROOT", "LINUX", "MOUNTS", "HOSTNAME_FIELD", "HOOKS", "ANNOTATIONS"],
    lambda version, platform, process, root, linux, mounts, hostname, hooks, annotations: b"{%s,%s,%s,%s,%s,%s,%s,%s,%s}"
    % (version, platform, process, root, linux, mounts, hostname, hooks, annotations),
)

ctx.rule("OCI_VERSION", '"ociVersion": "{VERSION_NUMBER}"')
ctx.rule("VERSION_NUMBER", "1.0.0")
ctx.rule("VERSION_NUMBER", "1.1.0")
ctx.rule("VERSION_NUMBER", "1.2.0")
ctx.regex("VERSION_NUMBER", "[0-9]+\\.[0-9]+\\.[0-9]+")

ctx.script(
    "PLATFORM",
    ["OS_NAME", "ARCH_NAME"],
    lambda os, arch: b'"platform": {"os": "%s","arch": "%s"}' % (os, arch),
)
ctx.rule("OS_NAME", "linux")
ctx.rule("ARCH_NAME", "amd64")
ctx.rule("ARCH_NAME", "arm64")

# Hostname / domainname
ctx.rule("HOSTNAME_FIELD", '"hostname": "container"')
ctx.rule("HOSTNAME_FIELD", '"hostname": "fuzztarget"')
ctx.rule("HOSTNAME_FIELD", '"hostname": "test-host"')
ctx.script(
    "HOSTNAME_FIELD",
    ["HOSTNAME_VAL", "DOMAINNAME_VAL"],
    lambda h, d: b'"hostname": "%s","domainName": "%s"' % (h, d),
)
ctx.rule("HOSTNAME_VAL", "mycontainer")
ctx.rule("HOSTNAME_VAL", "fuzzer")
ctx.rule("HOSTNAME_VAL", "test")
ctx.rule("DOMAINNAME_VAL", "local")
ctx.rule("DOMAINNAME_VAL", "example.com")

# =============================================================================
# PROCESS SECTION
# =============================================================================

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL"],
    lambda terminal, user, args, env, cwd: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s"}'
    % (terminal, user, args, env, cwd),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "CAPABILITIES"],
    lambda terminal, user, args, env, cwd, caps: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s}'
    % (terminal, user, args, env, cwd, caps),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "RLIMITS"],
    lambda terminal, user, args, env, cwd, rlimits: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s}'
    % (terminal, user, args, env, cwd, rlimits),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "NO_NEW_PRIVS"],
    lambda terminal, user, args, env, cwd, nnp: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s}'
    % (terminal, user, args, env, cwd, nnp),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "STOP_SIGNAL"],
    lambda terminal, user, args, env, cwd, sig: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s}'
    % (terminal, user, args, env, cwd, sig),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "CAPABILITIES", "RLIMITS"],
    lambda terminal, user, args, env, cwd, caps, rlimits: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s,%s}'
    % (terminal, user, args, env, cwd, caps, rlimits),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "CAPABILITIES", "RLIMITS", "NO_NEW_PRIVS"],
    lambda terminal, user, args, env, cwd, caps, rlimits, nnp: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s,%s,%s}'
    % (terminal, user, args, env, cwd, caps, rlimits, nnp),
)

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "CAPABILITIES", "NO_NEW_PRIVS", "STOP_SIGNAL"],
    lambda terminal, user, args, env, cwd, caps, nnp, sig: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s,%s,%s}'
    % (terminal, user, args, env, cwd, caps, nnp, sig),
)

ctx.rule("TERMINAL_VAL", "false")
ctx.rule("NO_NEW_PRIVS", '"noNewPrivileges": true')
ctx.rule("NO_NEW_PRIVS", '"noNewPrivileges": false')

ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGTERM"')
ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGKILL"')
ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGHUP"')
ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGINT"')
ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGUSR1"')
ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGUSR2"')
ctx.rule("STOP_SIGNAL", '"stopSignal": "SIGQUIT"')

ctx.script(
    "USER_CONFIG",
    ["UID_VAL", "GID_VAL"],
    lambda uid, gid: b'"user": {"uid": %s,"gid": %s}' % (uid, gid),
)
ctx.rule("UID_VAL", "0")
ctx.rule("UID_VAL", "1000")
ctx.rule("UID_VAL", "1001")
ctx.regex("UID_VAL", "[1-9][0-9]+")
ctx.rule("GID_VAL", "0")
ctx.rule("GID_VAL", "1000")
ctx.rule("GID_VAL", "1001")
ctx.regex("GID_VAL", "[1-9][0-9]+")

ctx.rule("ARGS_LIST", '"args": ["/bin/sh"]')
ctx.rule("ARGS_LIST", '"args": ["/bin/bash"]')
ctx.rule("ARGS_LIST", '"args": ["./app"]')
ctx.rule("ARGS_LIST", '"args": ["/usr/bin/nginx","-g","daemon off;"]')
ctx.rule("ARGS_LIST", '"args": ["/bin/true"]')
ctx.rule("ARGS_LIST", '"args": ["/app/app"]')

ctx.rule("ENV_LIST", '"env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"]')
ctx.rule("ENV_LIST", '"env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin","TERM=xterm"]')
ctx.rule("ENV_LIST", '"env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin","HOME=/root","USER=root"]')

ctx.rule("CWD_VAL", "/")
ctx.rule("CWD_VAL", "/app")
ctx.rule("CWD_VAL", "/home/user")
ctx.rule("CWD_VAL", "/root")

# =============================================================================
# CAPABILITIES
# =============================================================================

ctx.script(
    "CAPABILITIES",
    ["CAP_BOUNDING", "CAP_EFFECTIVE", "CAP_PERMITTED", "CAP_INHERITABLE", "CAP_AMBIENT"],
    lambda b, e, p, i, a: b'"capabilities": {"bounding": [%s],"effective": [%s],"permitted": [%s],"inheritable": [%s],"ambient": [%s]}'
    % (b, e, p, i, a),
)

ctx.script(
    "CAPABILITIES",
    ["CAP_BOUNDING", "CAP_EFFECTIVE", "CAP_PERMITTED"],
    lambda b, e, p: b'"capabilities": {"bounding": [%s],"effective": [%s],"permitted": [%s],"inheritable": [],"ambient": []}'
    % (b, e, p),
)

ctx.rule("CAPABILITIES", '"capabilities": \{"bounding": [],"effective": [],"permitted": [],"inheritable": [],"ambient": []\}')

ctx.script("CAP_BOUNDING",    ["CAP_LIST"], lambda c: c)
ctx.script("CAP_EFFECTIVE",   ["CAP_LIST"], lambda c: c)
ctx.script("CAP_PERMITTED",   ["CAP_LIST"], lambda c: c)
ctx.script("CAP_INHERITABLE", ["CAP_LIST"], lambda c: c)
ctx.script("CAP_AMBIENT",     ["CAP_LIST"], lambda c: c)

ctx.script("CAP_LIST", ["CAP_ITEM"], lambda c: c)
ctx.script("CAP_LIST", ["CAP_ITEM", "CAP_LIST"], lambda c, rest: b'%s,%s' % (c, rest))

ctx.rule("CAP_ITEM", '"CAP_CHOWN"')
ctx.rule("CAP_ITEM", '"CAP_DAC_OVERRIDE"')
ctx.rule("CAP_ITEM", '"CAP_DAC_READ_SEARCH"')
ctx.rule("CAP_ITEM", '"CAP_FSETID"')
ctx.rule("CAP_ITEM", '"CAP_FOWNER"')
ctx.rule("CAP_ITEM", '"CAP_MKNOD"')
ctx.rule("CAP_ITEM", '"CAP_NET_RAW"')
ctx.rule("CAP_ITEM", '"CAP_SETGID"')
ctx.rule("CAP_ITEM", '"CAP_SETUID"')
ctx.rule("CAP_ITEM", '"CAP_SETFCAP"')
ctx.rule("CAP_ITEM", '"CAP_SETPCAP"')
ctx.rule("CAP_ITEM", '"CAP_NET_BIND_SERVICE"')
ctx.rule("CAP_ITEM", '"CAP_SYS_CHROOT"')
ctx.rule("CAP_ITEM", '"CAP_KILL"')
ctx.rule("CAP_ITEM", '"CAP_AUDIT_WRITE"')
ctx.rule("CAP_ITEM", '"CAP_SYS_PTRACE"')
ctx.rule("CAP_ITEM", '"CAP_NET_ADMIN"')
ctx.rule("CAP_ITEM", '"CAP_SYS_ADMIN"')
ctx.rule("CAP_ITEM", '"CAP_SYS_RAWIO"')
ctx.rule("CAP_ITEM", '"CAP_SYS_MODULE"')

# =============================================================================
# RLIMITS
# =============================================================================

ctx.script("RLIMITS", ["RLIMIT_LIST"], lambda r: b'"rlimits": [%s]' % r)

ctx.script("RLIMIT_LIST", ["RLIMIT_ITEM"], lambda r: r)
ctx.script("RLIMIT_LIST", ["RLIMIT_ITEM", "RLIMIT_LIST"], lambda r, rest: b'%s,%s' % (r, rest))

ctx.script(
    "RLIMIT_ITEM",
    ["RLIMIT_TYPE", "RLIMIT_HARD", "RLIMIT_SOFT"],
    lambda t, hard, soft: b'{"type": "%s","hard": %s,"soft": %s}' % (t, hard, soft),
)

ctx.rule("RLIMIT_TYPE", "RLIMIT_NOFILE")
ctx.rule("RLIMIT_TYPE", "RLIMIT_NPROC")
ctx.rule("RLIMIT_TYPE", "RLIMIT_CORE")
ctx.rule("RLIMIT_TYPE", "RLIMIT_CPU")
ctx.rule("RLIMIT_TYPE", "RLIMIT_DATA")
ctx.rule("RLIMIT_TYPE", "RLIMIT_FSIZE")
ctx.rule("RLIMIT_TYPE", "RLIMIT_MEMLOCK")
ctx.rule("RLIMIT_TYPE", "RLIMIT_MSGQUEUE")
ctx.rule("RLIMIT_TYPE", "RLIMIT_NICE")
ctx.rule("RLIMIT_TYPE", "RLIMIT_RSS")
ctx.rule("RLIMIT_TYPE", "RLIMIT_RTPRIO")
ctx.rule("RLIMIT_TYPE", "RLIMIT_SIGPENDING")
ctx.rule("RLIMIT_TYPE", "RLIMIT_STACK")

ctx.rule("RLIMIT_HARD", "1024")
ctx.rule("RLIMIT_HARD", "4096")
ctx.rule("RLIMIT_HARD", "65536")
ctx.rule("RLIMIT_HARD", "0")
ctx.rule("RLIMIT_HARD", "-1")
ctx.regex("RLIMIT_HARD", "[0-9]+")

ctx.rule("RLIMIT_SOFT", "512")
ctx.rule("RLIMIT_SOFT", "1024")
ctx.rule("RLIMIT_SOFT", "0")
ctx.rule("RLIMIT_SOFT", "-1")
ctx.regex("RLIMIT_SOFT", "[0-9]+")
# keep 0/tiny: useful error paths for most limits, and a 0 RLIMIT_CPU just
# SIGKILLs (the harness drops that).

# =============================================================================
# ROOT FIELD
# =============================================================================

ctx.script(
    "ROOT",
    ["ROOT_PATH", "READONLY_VAL"],
    lambda path, readonly: b'"root": {"path": "%s","readonly": %s}' % (path, readonly),
)
ctx.rule("ROOT_PATH", "rootfs")
ctx.rule("READONLY_VAL", "false")
ctx.rule("READONLY_VAL", "true")

# =============================================================================
# LINUX SECTION
# =============================================================================

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "SECCOMP", "SYSCTL", "PATH_SECURITY"],
    lambda ns, res, sec, sysctl, paths: b'"linux": {%s,%s,%s,%s,%s}'
    % (ns, res, sec, sysctl, paths),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "DEVICES", "CGROUPS_PATH"],
    lambda ns, dev, cpath: b'"linux": {%s,%s,%s}' % (ns, dev, cpath),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "CGROUPS_PATH"],
    lambda ns, res, cpath: b'"linux": {%s,%s,%s}' % (ns, res, cpath),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "SECCOMP", "CGROUPS_PATH"],
    lambda ns, res, sec, cpath: b'"linux": {%s,%s,%s,%s}' % (ns, res, sec, cpath),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "SYSCTL", "CGROUPS_PATH"],
    lambda ns, res, sysctl, cpath: b'"linux": {%s,%s,%s,%s}' % (ns, res, sysctl, cpath),
)

ctx.script(
    "LINUX",
    ["RESOURCES", "ID_MAPPINGS", "ROOTFS_PROPAGATION"],
    lambda res, idmap, rfsprop: b'"linux": {%s,%s,%s}' % (res, idmap, rfsprop),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "SECCOMP", "SYSCTL"],
    lambda ns, res, sec, sysctl: b'"linux": {%s,%s,%s,%s}' % (ns, res, sec, sysctl),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "PATH_SECURITY"],
    lambda ns, res, paths: b'"linux": {%s,%s,%s}' % (ns, res, paths),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "SECCOMP", "PATH_SECURITY"],
    lambda ns, sec, paths: b'"linux": {%s,%s,%s}' % (ns, sec, paths),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "SECCOMP"],
    lambda ns, res, sec: b'"linux": {%s,%s,%s}' % (ns, res, sec),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "SYSCTL", "PATH_SECURITY"],
    lambda ns, sysctl, paths: b'"linux": {%s,%s,%s}' % (ns, sysctl, paths),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "ID_MAPPINGS", "RESOURCES"],
    lambda ns, idmap, res: b'"linux": {%s,%s,%s}' % (ns, idmap, res),
)

ctx.script(
    "LINUX",
    ["NAMESPACES", "RESOURCES", "SECCOMP", "SYSCTL", "PATH_SECURITY", "ROOTFS_PROPAGATION"],
    lambda ns, res, sec, sysctl, paths, prop: b'"linux": {%s,%s,%s,%s,%s,%s}'
    % (ns, res, sec, sysctl, paths, prop),
)

# =============================================================================
# NAMESPACES
# =============================================================================

ctx.rule("NAMESPACES", '"namespaces": []')
ctx.script("NAMESPACES", ["NAMESPACE_LIST"], lambda nslist: b'"namespaces": [%s]' % nslist)

ctx.script("NAMESPACE_LIST", ["NAMESPACE_ITEM"], lambda ns: ns)
ctx.script("NAMESPACE_LIST", ["NAMESPACE_ITEM", "NAMESPACE_LIST"],
    lambda ns1, nslist: b'%s,%s' % (ns1, nslist))

ctx.script("NAMESPACE_ITEM", ["NAMESPACE_TYPE"], lambda nstype: b'{"type": "%s"}' % nstype)

ctx.rule("NAMESPACE_TYPE", "pid")
ctx.rule("NAMESPACE_TYPE", "network")
ctx.rule("NAMESPACE_TYPE", "mount")
ctx.rule("NAMESPACE_TYPE", "ipc")
ctx.rule("NAMESPACE_TYPE", "uts")
ctx.rule("NAMESPACE_TYPE", "user")
ctx.rule("NAMESPACE_TYPE", "cgroup")
ctx.rule("NAMESPACE_TYPE", "time")

# =============================================================================
# RESOURCES
# =============================================================================

ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "CPU_LIMIT", "PIDS_LIMIT"],
    lambda mem, cpu, pids: b'"resources": {%s,%s,%s}' % (mem, cpu, pids),
)

ctx.script("RESOURCES", ["MEMORY_CONFIG"], lambda mem: b'"resources": {%s}' % mem)
ctx.script("RESOURCES", ["CPU_CONFIG"],    lambda cpu: b'"resources": {%s}' % cpu)

ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "CPU_LIMIT", "PIDS_LIMIT", "RESOURCES_DEVICES"],
    lambda mem, cpu, pids, devs: b'"resources": {%s,%s,%s,%s}' % (mem, cpu, pids, devs),
)

ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "CPU_LIMIT", "PIDS_LIMIT", "BLOCK_IO"],
    lambda mem, cpu, pids, bio: b'"resources": {%s,%s,%s,%s}' % (mem, cpu, pids, bio),
)

ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "CPU_LIMIT", "PIDS_LIMIT", "RESOURCES_DEVICES", "BLOCK_IO"],
    lambda mem, cpu, pids, devs, bio: b'"resources": {%s,%s,%s,%s,%s}' % (mem, cpu, pids, devs, bio),
)

ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "PIDS_LIMIT", "RESOURCES_DEVICES"],
    lambda mem, pids, devs: b'"resources": {%s,%s,%s}' % (mem, pids, devs),
)

ctx.script(
    "RESOURCES",
    ["CPU_CONFIG", "PIDS_LIMIT"],
    lambda cpu, pids: b'"resources": {%s,%s}' % (cpu, pids),
)

ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "CPU_LIMIT", "PIDS_LIMIT", "HUGEPAGE_LIMITS"],
    lambda mem, cpu, pids, huge: b'"resources": {%s,%s,%s,%s}' % (mem, cpu, pids, huge),
)

# Memory
ctx.script("MEMORY_CONFIG", ["MEMORY_LIMIT"],
    lambda limit: b'"memory": {"limit": %s}' % limit)
ctx.script("MEMORY_CONFIG", ["MEMORY_LIMIT", "MEMORY_SWAP"],
    lambda limit, swap: b'"memory": {"limit": %s,"swap": %s}' % (limit, swap))
ctx.script("MEMORY_CONFIG", ["MEMORY_LIMIT", "MEMORY_RESERVATION", "MEMORY_SWAPPINESS"],
    lambda limit, res, swap: b'"memory": {"limit": %s,"reservation": %s,"swappiness": %s}' % (limit, res, swap))
ctx.script("MEMORY_CONFIG", ["MEMORY_LIMIT", "MEMORY_SWAP", "MEMORY_SWAPPINESS"],
    lambda limit, swap, swappy: b'"memory": {"limit": %s,"swap": %s,"swappiness": %s,"disableOOMKiller": false}' % (limit, swap, swappy))

# floor memory.limit (crun joins the cgroup before exec; too small OOM-kills it)
ctx.rule("MEMORY_LIMIT", "134217728")
ctx.rule("MEMORY_LIMIT", "268435456")
ctx.rule("MEMORY_LIMIT", "536870912")
ctx.rule("MEMORY_LIMIT", "1073741824")
ctx.rule("MEMORY_LIMIT", "-1")
ctx.rule("MEMORY_SWAP", "268435456")
ctx.rule("MEMORY_SWAP", "536870912")
ctx.rule("MEMORY_SWAP", "1073741824")
ctx.rule("MEMORY_SWAP", "-1")
ctx.rule("MEMORY_RESERVATION", "67108864")
ctx.rule("MEMORY_RESERVATION", "134217728")
ctx.rule("MEMORY_SWAPPINESS", "60")
ctx.rule("MEMORY_SWAPPINESS", "10")
ctx.rule("MEMORY_SWAPPINESS", "0")
ctx.rule("MEMORY_SWAPPINESS", "100")

# CPU
ctx.script("CPU_CONFIG", ["CPU_SHARES"],
    lambda shares: b'"cpu": {"shares": %s}' % shares)
ctx.script("CPU_CONFIG", ["CPU_SHARES", "CPU_PERIOD", "CPU_QUOTA"],
    lambda shares, period, quota: b'"cpu": {"shares": %s,"period": %s,"quota": %s}' % (shares, period, quota))
ctx.script("CPU_CONFIG", ["CPU_CPUS"],
    lambda cpus: b'"cpu": {"cpus": "%s"}' % cpus)
ctx.script("CPU_CONFIG", ["CPU_SHARES", "CPU_CPUS"],
    lambda shares, cpus: b'"cpu": {"shares": %s,"cpus": "%s"}' % (shares, cpus))

ctx.rule("CPU_LIMIT", '"cpu": \{"shares": 1024\}')
ctx.rule("CPU_LIMIT", '"cpu": \{"shares": 512\}')
ctx.rule("CPU_LIMIT", '"cpu": \{"period": 100000,"quota": 50000\}')
ctx.rule("CPU_LIMIT", '"cpu": \{"period": 100000,"quota": -1\}')
ctx.rule("CPU_SHARES", "1024")
ctx.rule("CPU_SHARES", "512")
ctx.rule("CPU_SHARES", "256")
ctx.rule("CPU_SHARES", "2048")
ctx.rule("CPU_PERIOD", "100000")
ctx.rule("CPU_PERIOD", "50000")
ctx.rule("CPU_QUOTA", "50000")
ctx.rule("CPU_QUOTA", "25000")
ctx.rule("CPU_QUOTA", "-1")
ctx.rule("CPU_CPUS", "0-1")
ctx.rule("CPU_CPUS", "0,2")
ctx.rule("CPU_CPUS", "0-3")
ctx.rule("CPU_CPUS", "0")

# PIDs
ctx.rule("PIDS_LIMIT", '"pids": \{"limit": 1024\}')
ctx.rule("PIDS_LIMIT", '"pids": \{"limit": 512\}')
ctx.rule("PIDS_LIMIT", '"pids": \{"limit": 2048\}')
ctx.rule("PIDS_LIMIT", '"pids": \{"limit": 100\}')
ctx.rule("PIDS_LIMIT", '"pids": \{"limit": -1\}')

# Devices under resources (cgroup device allow/deny rules)
ctx.script("RESOURCES_DEVICES", ["RESOURCE_DEVICE_LIST"],
    lambda devs: b'"devices": [%s]' % devs)
ctx.rule("RESOURCES_DEVICES", '"devices": [\{"allow": false,"access": "rwm"\}]')
ctx.rule("RESOURCES_DEVICES", '"devices": [\{"allow": true,"access": "rwm"\}]')

ctx.script("RESOURCE_DEVICE_LIST", ["RESOURCE_DEVICE_ITEM"], lambda d: d)
ctx.script("RESOURCE_DEVICE_LIST", ["RESOURCE_DEVICE_ITEM", "RESOURCE_DEVICE_LIST"],
    lambda d, rest: b'%s,%s' % (d, rest))

ctx.script(
    "RESOURCE_DEVICE_ITEM",
    ["RD_ALLOW", "RD_TYPE", "RD_MAJOR", "RD_MINOR", "RD_ACCESS"],
    lambda allow, dtype, major, minor, access:
        b'{"allow": %s,"type": "%s","major": %s,"minor": %s,"access": "%s"}' % (allow, dtype, major, minor, access),
)
ctx.rule("RESOURCE_DEVICE_ITEM", '\{"allow": false,"access": "rwm"\}')
ctx.rule("RESOURCE_DEVICE_ITEM", '\{"allow": true,"type": "c","major": 1,"minor": 3,"access": "rwm"\}')
ctx.rule("RESOURCE_DEVICE_ITEM", '\{"allow": true,"type": "c","major": 1,"minor": 8,"access": "r"\}')
ctx.rule("RESOURCE_DEVICE_ITEM", '\{"allow": true,"type": "c","major": 5,"minor": 0,"access": "rw"\}')

ctx.rule("RD_ALLOW", "true")
ctx.rule("RD_ALLOW", "false")
ctx.rule("RD_TYPE", "c")
ctx.rule("RD_TYPE", "b")
ctx.rule("RD_TYPE", "a")
ctx.rule("RD_MAJOR", "1")
ctx.rule("RD_MAJOR", "5")
ctx.rule("RD_MAJOR", "8")
ctx.rule("RD_MAJOR", "10")
ctx.rule("RD_MINOR", "0")
ctx.rule("RD_MINOR", "3")
ctx.rule("RD_MINOR", "5")
ctx.rule("RD_MINOR", "9")
ctx.rule("RD_ACCESS", "rwm")
ctx.rule("RD_ACCESS", "rw")
ctx.rule("RD_ACCESS", "r")
ctx.rule("RD_ACCESS", "wm")

# BlockIO
ctx.script("BLOCK_IO", ["BLOCK_IO_WEIGHT"],
    lambda w: b'"blockIO": {"weight": %s}' % w)
ctx.script("BLOCK_IO", ["BLOCK_IO_WEIGHT", "BLOCK_IO_LEAF"],
    lambda w, l: b'"blockIO": {"weight": %s,"leafWeight": %s}' % (w, l))
ctx.rule("BLOCK_IO_WEIGHT", "100")
ctx.rule("BLOCK_IO_WEIGHT", "500")
ctx.rule("BLOCK_IO_WEIGHT", "1000")
ctx.rule("BLOCK_IO_WEIGHT", "10")
ctx.rule("BLOCK_IO_LEAF", "100")
ctx.rule("BLOCK_IO_LEAF", "500")

# Hugepage limits
ctx.script("HUGEPAGE_LIMITS", ["HUGEPAGE_ITEM"],
    lambda h: b'"hugepageLimits": [%s]' % h)
ctx.rule("HUGEPAGE_ITEM", '\{"pageSize": "2MB","limit": 0\}')
ctx.rule("HUGEPAGE_ITEM", '\{"pageSize": "1GB","limit": 0\}')
ctx.rule("HUGEPAGE_ITEM", '\{"pageSize": "2MB","limit": 134217728\}')

# =============================================================================
# SECCOMP
# =============================================================================

# Simple default-action only
ctx.rule("SECCOMP", '"seccomp": \{"defaultAction": "SCMP_ACT_ALLOW"\}')
ctx.rule("SECCOMP", '"seccomp": \{"defaultAction": "SCMP_ACT_ERRNO"\}')
ctx.rule("SECCOMP", '"seccomp": \{"defaultAction": "SCMP_ACT_KILL"\}')
ctx.rule("SECCOMP", '"seccomp": \{"defaultAction": "SCMP_ACT_KILL_PROCESS"\}')
ctx.rule("SECCOMP", '"seccomp": \{"defaultAction": "SCMP_ACT_LOG"\}')

# Safe seccomp rules: defaultAction=ALLOW so container can run successfully,
# but specific syscall rules with ERRNO/KILL/LOG force get_seccomp_action and
# get_seccomp_operator to be called in the parent during filter building.
# These go to corpus (not crashes) and exercise seccomp code that ERRNO-default
# configs can't reach because the container dies before the filter is useful.
ctx.rule("SECCOMP",
    '"seccomp": \{"defaultAction": "SCMP_ACT_ALLOW","syscalls": ['
    '\{"names": ["ptrace"],"action": "SCMP_ACT_ERRNO"\},'
    '\{"names": ["keyctl"],"action": "SCMP_ACT_KILL"\},'
    '\{"names": ["prctl"],"action": "SCMP_ACT_LOG"\}'
    ']\}')
ctx.rule("SECCOMP",
    '"seccomp": \{"defaultAction": "SCMP_ACT_ALLOW","syscalls": ['
    '\{"names": ["mount","umount2"],"action": "SCMP_ACT_ERRNO"\},'
    '\{"names": ["pivot_root"],"action": "SCMP_ACT_ERRNO"\},'
    '\{"names": ["unshare"],"action": "SCMP_ACT_LOG"\}'
    ']\}')
ctx.rule("SECCOMP",
    '"seccomp": \{"defaultAction": "SCMP_ACT_ALLOW","syscalls": ['
    '\{"names": ["socket","bind","listen"],"action": "SCMP_ACT_ERRNO"\},'
    '\{"names": ["setuid","setgid"],"action": "SCMP_ACT_LOG"\}'
    ']\}')
ctx.rule("SECCOMP",
    '"seccomp": \{"defaultAction": "SCMP_ACT_LOG","syscalls": ['
    '\{"names": ["execve","execveat"],"action": "SCMP_ACT_ALLOW"\},'
    '\{"names": ["exit","exit_group"],"action": "SCMP_ACT_ALLOW"\},'
    '\{"names": ["ptrace"],"action": "SCMP_ACT_KILL"\}'
    ']\}')

# Default action + syscall rules
ctx.script(
    "SECCOMP",
    ["SECCOMP_DEFAULT", "SECCOMP_SYSCALLS"],
    lambda default, syscalls: b'"seccomp": {"defaultAction": "%s",%s}' % (default, syscalls),
)

# Default action + architectures + syscall rules
ctx.script(
    "SECCOMP",
    ["SECCOMP_DEFAULT", "SECCOMP_ARCHS", "SECCOMP_SYSCALLS"],
    lambda default, archs, syscalls: b'"seccomp": {"defaultAction": "%s",%s,%s}' % (default, archs, syscalls),
)

# Default action + architectures only
ctx.script(
    "SECCOMP",
    ["SECCOMP_DEFAULT", "SECCOMP_ARCHS"],
    lambda default, archs: b'"seccomp": {"defaultAction": "%s",%s}' % (default, archs),
)

ctx.rule("SECCOMP_DEFAULT", "SCMP_ACT_ALLOW")
ctx.rule("SECCOMP_DEFAULT", "SCMP_ACT_ERRNO")
ctx.rule("SECCOMP_DEFAULT", "SCMP_ACT_KILL")
ctx.rule("SECCOMP_DEFAULT", "SCMP_ACT_KILL_PROCESS")
ctx.rule("SECCOMP_DEFAULT", "SCMP_ACT_LOG")
ctx.rule("SECCOMP_DEFAULT", "SCMP_ACT_TRACE")

# Architectures
ctx.script("SECCOMP_ARCHS", ["SECCOMP_ARCH_LIST"],
    lambda a: b'"architectures": [%s]' % a)
ctx.rule("SECCOMP_ARCH_LIST", '"SCMP_ARCH_X86_64"')
ctx.rule("SECCOMP_ARCH_LIST", '"SCMP_ARCH_X86_64","SCMP_ARCH_X86"')
ctx.rule("SECCOMP_ARCH_LIST", '"SCMP_ARCH_X86_64","SCMP_ARCH_AARCH64"')
ctx.rule("SECCOMP_ARCH_LIST", '"SCMP_ARCH_X86"')
ctx.rule("SECCOMP_ARCH_LIST", '"SCMP_ARCH_AARCH64"')

# Syscall rules
ctx.script("SECCOMP_SYSCALLS", ["SYSCALL_LIST"],
    lambda syscalls: b'"syscalls": [%s]' % syscalls)

ctx.script("SYSCALL_LIST", ["SYSCALL_ITEM"], lambda syscall: syscall)
ctx.script("SYSCALL_LIST", ["SYSCALL_ITEM", "SYSCALL_LIST"],
    lambda s1, rest: b'%s,%s' % (s1, rest))

# Syscall item without arg filters
ctx.script(
    "SYSCALL_ITEM",
    ["SYSCALL_NAMES", "SYSCALL_ACTION"],
    lambda names, action: b'{"names": [%s],"action": "%s"}' % (names, action),
)

# Syscall item with arg filters
ctx.script(
    "SYSCALL_ITEM",
    ["SYSCALL_NAMES", "SYSCALL_ACTION", "SYSCALL_ARGS"],
    lambda names, action, args: b'{"names": [%s],"action": "%s",%s}' % (names, action, args),
)

ctx.rule("SYSCALL_NAMES", '"open","openat"')
ctx.rule("SYSCALL_NAMES", '"write","read"')
ctx.rule("SYSCALL_NAMES", '"mmap","munmap"')
ctx.rule("SYSCALL_NAMES", '"socket","bind","listen"')
ctx.rule("SYSCALL_NAMES", '"execve","execveat"')
ctx.rule("SYSCALL_NAMES", '"fork","clone","clone3"')
ctx.rule("SYSCALL_NAMES", '"kill","tkill","tgkill"')
ctx.rule("SYSCALL_NAMES", '"ptrace"')
ctx.rule("SYSCALL_NAMES", '"ioctl"')
ctx.rule("SYSCALL_NAMES", '"prctl"')
ctx.rule("SYSCALL_NAMES", '"setuid","setgid","setresuid","setresgid"')
ctx.rule("SYSCALL_NAMES", '"mprotect","mmap","mmap2"')
ctx.rule("SYSCALL_NAMES", '"open","openat","openat2","open_by_handle_at"')
ctx.rule("SYSCALL_NAMES", '"read","write","readv","writev","pread64","pwrite64"')
ctx.rule("SYSCALL_NAMES", '"exit","exit_group"')
ctx.rule("SYSCALL_NAMES", '"brk"')
ctx.rule("SYSCALL_NAMES", '"connect","sendto","recvfrom","sendmsg","recvmsg"')
ctx.rule("SYSCALL_NAMES", '"mount","umount","umount2"')
ctx.rule("SYSCALL_NAMES", '"unshare"')
ctx.rule("SYSCALL_NAMES", '"setns"')
ctx.rule("SYSCALL_NAMES", '"pivot_root"')
ctx.rule("SYSCALL_NAMES", '"capset","capget"')
ctx.rule("SYSCALL_NAMES", '"keyctl"')

ctx.rule("SYSCALL_ACTION", "SCMP_ACT_ALLOW")
ctx.rule("SYSCALL_ACTION", "SCMP_ACT_ERRNO")
ctx.rule("SYSCALL_ACTION", "SCMP_ACT_KILL")
ctx.rule("SYSCALL_ACTION", "SCMP_ACT_KILL_PROCESS")
ctx.rule("SYSCALL_ACTION", "SCMP_ACT_NOTIFY")
ctx.rule("SYSCALL_ACTION", "SCMP_ACT_LOG")
ctx.rule("SYSCALL_ACTION", "SCMP_ACT_TRACE")

# Syscall arg filters
ctx.script("SYSCALL_ARGS", ["SYSCALL_ARG_LIST"],
    lambda args: b'"args": [%s]' % args)

ctx.script("SYSCALL_ARG_LIST", ["SYSCALL_ARG_ITEM"], lambda arg: arg)
ctx.script("SYSCALL_ARG_LIST", ["SYSCALL_ARG_ITEM", "SYSCALL_ARG_LIST"],
    lambda arg, rest: b'%s,%s' % (arg, rest))

ctx.script(
    "SYSCALL_ARG_ITEM",
    ["ARG_INDEX", "ARG_VALUE", "ARG_OP"],
    lambda idx, val, op: b'{"index": %s,"value": %s,"op": "%s"}' % (idx, val, op),
)

# MASKED_EQ requires a valueTwo field
ctx.script(
    "SYSCALL_ARG_ITEM",
    ["ARG_INDEX", "ARG_VALUE", "ARG_VALUE2", "ARG_OP_MASKED"],
    lambda idx, val, val2, op: b'{"index": %s,"value": %s,"valueTwo": %s,"op": "%s"}' % (idx, val, val2, op),
)

ctx.rule("ARG_INDEX", "0")
ctx.rule("ARG_INDEX", "1")
ctx.rule("ARG_INDEX", "2")
ctx.rule("ARG_INDEX", "3")
ctx.rule("ARG_INDEX", "4")
ctx.rule("ARG_INDEX", "5")

ctx.rule("ARG_VALUE", "0")
ctx.rule("ARG_VALUE", "1")
ctx.rule("ARG_VALUE", "2")
ctx.rule("ARG_VALUE", "4096")
ctx.rule("ARG_VALUE", "65536")
ctx.rule("ARG_VALUE", "4294967295")
ctx.regex("ARG_VALUE", "[0-9]+")

ctx.rule("ARG_VALUE2", "0")
ctx.rule("ARG_VALUE2", "255")
ctx.rule("ARG_VALUE2", "4294967295")
ctx.regex("ARG_VALUE2", "[0-9]+")

ctx.rule("ARG_OP", "SCMP_CMP_NE")
ctx.rule("ARG_OP", "SCMP_CMP_LT")
ctx.rule("ARG_OP", "SCMP_CMP_LE")
ctx.rule("ARG_OP", "SCMP_CMP_EQ")
ctx.rule("ARG_OP", "SCMP_CMP_GE")
ctx.rule("ARG_OP", "SCMP_CMP_GT")
ctx.rule("ARG_OP_MASKED", "SCMP_CMP_MASKED_EQ")

# =============================================================================
# SYSCTL
# =============================================================================

ctx.rule("SYSCTL", '"sysctl": \{\}')
ctx.script("SYSCTL", ["SYSCTL_ITEMS"], lambda items: b'"sysctl": {%s}' % items)

ctx.rule("SYSCTL_ITEMS", '"net.ipv4.ip_forward": "1"')
ctx.rule("SYSCTL_ITEMS", '"kernel.shm_rmid_forced": "1"')
ctx.rule("SYSCTL_ITEMS", '"net.ipv4.ping_group_range": "0 2147483647"')
ctx.rule("SYSCTL_ITEMS", '"kernel.msgmax": "65536"')
ctx.rule("SYSCTL_ITEMS", '"kernel.shmmax": "67108864"')
ctx.rule("SYSCTL_ITEMS", '"kernel.shmall": "4294967296"')
ctx.rule("SYSCTL_ITEMS", '"net.core.somaxconn": "128"')
ctx.rule("SYSCTL_ITEMS", '"net.ipv4.conf.all.forwarding": "1"')
ctx.rule("SYSCTL_ITEMS", '"net.ipv4.tcp_syncookies": "1"')
ctx.rule("SYSCTL_ITEMS", '"net.ipv4.ip_forward": "1","kernel.shm_rmid_forced": "1"')
ctx.rule("SYSCTL_ITEMS", '"kernel.msgmax": "65536","net.core.somaxconn": "128"')
ctx.rule("SYSCTL_ITEMS", '"kernel.shmmax": "67108864","kernel.shmall": "4294967296"')
ctx.rule("SYSCTL_ITEMS", '"net.ipv4.conf.all.forwarding": "1","net.ipv4.ip_forward": "1"')
ctx.rule("SYSCTL_ITEMS", '"net.ipv4.tcp_syncookies": "1","net.ipv4.ip_forward": "0"')

# =============================================================================
# PATH SECURITY
# =============================================================================

ctx.script(
    "PATH_SECURITY",
    ["MASKED_PATHS", "READONLY_PATHS"],
    lambda masked, readonly: b'%s,%s' % (masked, readonly),
)
ctx.rule("PATH_SECURITY", '"maskedPaths": ["/proc/kcore","/proc/latency_stats"]')
ctx.rule("PATH_SECURITY", '"readonlyPaths": ["/proc/sys","/proc/sysrq-trigger"]')

ctx.rule("MASKED_PATHS", '"maskedPaths": []')
ctx.rule("MASKED_PATHS", '"maskedPaths": ["/proc/kcore"]')
ctx.rule("MASKED_PATHS", '"maskedPaths": ["/proc/kcore","/proc/latency_stats","/proc/timer_stats"]')
ctx.rule("MASKED_PATHS", '"maskedPaths": ["/proc/kcore","/proc/sysrq-trigger","/proc/irq","/proc/bus","/proc/sys/fs/binfmt_misc"]')

ctx.rule("READONLY_PATHS", '"readonlyPaths": []')
ctx.rule("READONLY_PATHS", '"readonlyPaths": ["/proc/sys"]')
ctx.rule("READONLY_PATHS", '"readonlyPaths": ["/proc/sys","/proc/sysrq-trigger","/proc/irq","/proc/bus"]')

# =============================================================================
# HOOKS
# =============================================================================

ctx.script("HOOKS", ["HOOK_PRESTART"],
    lambda pre: b'"hooks": {%s}' % pre)
ctx.script("HOOKS", ["HOOK_POSTSTART"],
    lambda post: b'"hooks": {%s}' % post)
ctx.script("HOOKS", ["HOOK_POSTSTOP"],
    lambda stop: b'"hooks": {%s}' % stop)
ctx.script("HOOKS", ["HOOK_CREATE_RUNTIME"],
    lambda cr: b'"hooks": {%s}' % cr)
ctx.script("HOOKS", ["HOOK_CREATE_CONTAINER"],
    lambda cc: b'"hooks": {%s}' % cc)
ctx.script("HOOKS", ["HOOK_START_CONTAINER"],
    lambda sc: b'"hooks": {%s}' % sc)
ctx.script("HOOKS", ["HOOK_PRESTART", "HOOK_POSTSTART"],
    lambda pre, post: b'"hooks": {%s,%s}' % (pre, post))
ctx.script("HOOKS", ["HOOK_PRESTART", "HOOK_POSTSTOP"],
    lambda pre, stop: b'"hooks": {%s,%s}' % (pre, stop))
ctx.script("HOOKS", ["HOOK_CREATE_RUNTIME", "HOOK_CREATE_CONTAINER", "HOOK_START_CONTAINER"],
    lambda cr, cc, sc: b'"hooks": {%s,%s,%s}' % (cr, cc, sc))
ctx.script("HOOKS", ["HOOK_PRESTART", "HOOK_POSTSTART", "HOOK_POSTSTOP"],
    lambda pre, post, stop: b'"hooks": {%s,%s,%s}' % (pre, post, stop))
ctx.script("HOOKS", ["HOOK_CREATE_RUNTIME", "HOOK_POSTSTART", "HOOK_POSTSTOP"],
    lambda cr, post, stop: b'"hooks": {%s,%s,%s}' % (cr, post, stop))

ctx.script("HOOK_PRESTART",         ["HOOK_ENTRY_LIST"], lambda e: b'"prestart": [%s]' % e)
ctx.script("HOOK_POSTSTART",        ["HOOK_ENTRY_LIST"], lambda e: b'"poststart": [%s]' % e)
ctx.script("HOOK_POSTSTOP",         ["HOOK_ENTRY_LIST"], lambda e: b'"poststop": [%s]' % e)
ctx.script("HOOK_CREATE_RUNTIME",   ["HOOK_ENTRY_LIST"], lambda e: b'"createRuntime": [%s]' % e)
ctx.script("HOOK_CREATE_CONTAINER", ["HOOK_ENTRY_LIST"], lambda e: b'"createContainer": [%s]' % e)
ctx.script("HOOK_START_CONTAINER",  ["HOOK_ENTRY_LIST"], lambda e: b'"startContainer": [%s]' % e)

ctx.script("HOOK_ENTRY_LIST", ["HOOK_ENTRY"], lambda e: e)
ctx.script("HOOK_ENTRY_LIST", ["HOOK_ENTRY", "HOOK_ENTRY"],
    lambda e1, e2: b'%s,%s' % (e1, e2))

ctx.script("HOOK_ENTRY", ["HOOK_PATH"],
    lambda path: b'{"path": "%s"}' % path)
ctx.script("HOOK_ENTRY", ["HOOK_PATH", "HOOK_ARGS"],
    lambda path, args: b'{"path": "%s",%s}' % (path, args))
ctx.script("HOOK_ENTRY", ["HOOK_PATH", "HOOK_ARGS", "HOOK_TIMEOUT"],
    lambda path, args, timeout: b'{"path": "%s",%s,"timeout": %s}' % (path, args, timeout))
ctx.script("HOOK_ENTRY", ["HOOK_PATH", "HOOK_ENV"],
    lambda path, env: b'{"path": "%s",%s}' % (path, env))

ctx.rule("HOOK_PATH", "/bin/true")
ctx.rule("HOOK_PATH", "/bin/sh")
ctx.rule("HOOK_PATH", "/usr/bin/nginx")
ctx.rule("HOOK_PATH", "/app/app")

ctx.rule("HOOK_ARGS", '"args": ["/bin/true"]')
ctx.rule("HOOK_ARGS", '"args": ["/bin/sh","-c","echo hook"]')
ctx.script("HOOK_ARGS", ["HOOK_ARG_LIST"], lambda args: b'"args": [%s]' % args)

ctx.script("HOOK_ARG_LIST", ["HOOK_ARG_VAL"], lambda a: b'"%s"' % a)
ctx.script("HOOK_ARG_LIST", ["HOOK_ARG_VAL", "HOOK_ARG_LIST"],
    lambda a, rest: b'"%s",%s' % (a, rest))
ctx.rule("HOOK_ARG_VAL", "start")
ctx.rule("HOOK_ARG_VAL", "stop")
ctx.rule("HOOK_ARG_VAL", "arg1")

ctx.rule("HOOK_TIMEOUT", "5")
ctx.rule("HOOK_TIMEOUT", "10")
ctx.rule("HOOK_TIMEOUT", "30")

ctx.rule("HOOK_ENV", '"env": ["FOO=bar"]')
ctx.rule("HOOK_ENV", '"env": ["HOOK_TYPE=prestart","PATH=/usr/bin:/bin"]')

# =============================================================================
# MOUNTS
# =============================================================================

ctx.rule("MOUNTS", '"mounts": []')
ctx.script("MOUNTS", ["MOUNT_ITEM"], lambda mount: b'"mounts": [%s]' % mount)
ctx.script("MOUNTS", ["MOUNT_LIST"], lambda mounts: b'"mounts": [%s]' % mounts)

ctx.script("MOUNT_LIST", ["MOUNT_ITEM", "MOUNT_ITEM"],
    lambda m1, m2: b'%s,%s' % (m1, m2))
ctx.script("MOUNT_LIST", ["MOUNT_ITEM", "MOUNT_ITEM", "MOUNT_ITEM"],
    lambda m1, m2, m3: b'%s,%s,%s' % (m1, m2, m3))

# Mount without options
ctx.script(
    "MOUNT_ITEM",
    ["MOUNT_DEST", "MOUNT_TYPE", "MOUNT_SRC"],
    lambda dest, mtype, src: b'{"destination": "%s","type": "%s","source": "%s"}'
    % (dest, mtype, src),
)

# Mount with options
ctx.script(
    "MOUNT_ITEM",
    ["MOUNT_DEST", "MOUNT_TYPE", "MOUNT_SRC", "MOUNT_OPTIONS"],
    lambda dest, mtype, src, opts: b'{"destination": "%s","type": "%s","source": "%s",%s}'
    % (dest, mtype, src, opts),
)

ctx.rule("MOUNT_DEST", "/proc")
ctx.rule("MOUNT_DEST", "/tmp")
ctx.rule("MOUNT_DEST", "/dev/shm")
ctx.rule("MOUNT_DEST", "/sys")
ctx.rule("MOUNT_DEST", "/dev")
ctx.rule("MOUNT_DEST", "/run")
ctx.rule("MOUNT_DEST", "/var/tmp")

ctx.rule("MOUNT_TYPE", "proc")
ctx.rule("MOUNT_TYPE", "tmpfs")
ctx.rule("MOUNT_TYPE", "sysfs")
ctx.rule("MOUNT_TYPE", "bind")
ctx.rule("MOUNT_TYPE", "devpts")
ctx.rule("MOUNT_TYPE", "mqueue")
ctx.rule("MOUNT_TYPE", "cgroup")
ctx.rule("MOUNT_TYPE", "cgroup2")

ctx.rule("MOUNT_SRC", "proc")
ctx.rule("MOUNT_SRC", "tmpfs")
ctx.rule("MOUNT_SRC", "sysfs")
ctx.rule("MOUNT_SRC", "/host/path")
# real host paths (launch_campaigns.sh creates them) so bind mounts resolve
ctx.rule("MOUNT_SRC", "/tmp/fuzz_bind_src")
ctx.rule("MOUNT_SRC", "/tmp/fuzz_bind_src/file.txt")
ctx.rule("MOUNT_SRC", "/tmp/fuzz_bind_src/dir")
ctx.rule("MOUNT_SRC", "devpts")
ctx.rule("MOUNT_SRC", "mqueue")
ctx.rule("MOUNT_SRC", "cgroup")

# Mount options
ctx.rule("MOUNT_OPTIONS", '"options": []')
ctx.rule("MOUNT_OPTIONS", '"options": ["ro","nosuid","nodev","noexec"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["nosuid","noexec","nodev","mode=755"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["rbind","rprivate"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["rbind","rshared"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["rbind","rslave"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["shared"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["slave"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["private"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["unbindable"]')
ctx.rule("MOUNT_OPTIONS", '"options": ["mode=1777","nosuid","nodev"]')
ctx.script("MOUNT_OPTIONS", ["MOUNT_OPTION_LIST"],
    lambda opts: b'"options": [%s]' % opts)

ctx.script("MOUNT_OPTION_LIST", ["MOUNT_OPTION_VAL"],
    lambda opt: b'"%s"' % opt)
ctx.script("MOUNT_OPTION_LIST", ["MOUNT_OPTION_VAL", "MOUNT_OPTION_LIST"],
    lambda opt, rest: b'"%s",%s' % (opt, rest))

ctx.rule("MOUNT_OPTION_VAL", "ro")
ctx.rule("MOUNT_OPTION_VAL", "rw")
ctx.rule("MOUNT_OPTION_VAL", "noexec")
ctx.rule("MOUNT_OPTION_VAL", "nosuid")
ctx.rule("MOUNT_OPTION_VAL", "nodev")
ctx.rule("MOUNT_OPTION_VAL", "rbind")
ctx.rule("MOUNT_OPTION_VAL", "bind")
ctx.rule("MOUNT_OPTION_VAL", "rprivate")
ctx.rule("MOUNT_OPTION_VAL", "rshared")
ctx.rule("MOUNT_OPTION_VAL", "rslave")
ctx.rule("MOUNT_OPTION_VAL", "shared")
ctx.rule("MOUNT_OPTION_VAL", "slave")
ctx.rule("MOUNT_OPTION_VAL", "private")
ctx.rule("MOUNT_OPTION_VAL", "unbindable")
ctx.rule("MOUNT_OPTION_VAL", "mode=755")
ctx.rule("MOUNT_OPTION_VAL", "mode=1777")
ctx.rule("MOUNT_OPTION_VAL", "size=65536k")

# =============================================================================
# DEVICES (top-level linux.devices — device nodes to create in rootfs)
# =============================================================================

ctx.rule("DEVICES", '"devices": []')
ctx.script("DEVICES", ["DEVICE_LIST"], lambda devlist: b'"devices": [%s]' % devlist)

ctx.script("DEVICE_LIST", ["DEVICE_ITEM"], lambda dev: dev)
ctx.script("DEVICE_LIST", ["DEVICE_ITEM", "DEVICE_LIST"],
    lambda dev1, devlist: b'%s,%s' % (dev1, devlist))

ctx.script(
    "DEVICE_ITEM",
    ["DEVICE_PATH", "DEVICE_TYPE", "DEVICE_MAJOR", "DEVICE_MINOR"],
    lambda path, dtype, major, minor:
        b'{"path": "%s","type": "%s","major": %s,"minor": %s}' % (path, dtype, major, minor),
)

ctx.rule("DEVICE_PATH", "/dev/null")
ctx.rule("DEVICE_PATH", "/dev/zero")
ctx.rule("DEVICE_PATH", "/dev/random")
ctx.rule("DEVICE_PATH", "/dev/urandom")
ctx.rule("DEVICE_PATH", "/dev/tty")
ctx.rule("DEVICE_PATH", "/dev/full")

ctx.rule("DEVICE_TYPE", "c")
ctx.rule("DEVICE_TYPE", "b")

ctx.rule("DEVICE_MAJOR", "1")
ctx.rule("DEVICE_MAJOR", "5")
ctx.rule("DEVICE_MAJOR", "8")
ctx.rule("DEVICE_MAJOR", "10")

ctx.rule("DEVICE_MINOR", "3")
ctx.rule("DEVICE_MINOR", "5")
ctx.rule("DEVICE_MINOR", "7")
ctx.rule("DEVICE_MINOR", "8")
ctx.rule("DEVICE_MINOR", "9")

# =============================================================================
# ID MAPPINGS
# =============================================================================

ctx.script("ID_MAPPINGS", ["UID_MAPPINGS", "GID_MAPPINGS"],
    lambda uid, gid: b'%s,%s' % (uid, gid))
ctx.rule("ID_MAPPINGS", '"uidMappings": [],"gidMappings": []')

ctx.script("UID_MAPPINGS", ["UID_MAPPING_LIST"],
    lambda mappings: b'"uidMappings": [%s]' % mappings)
ctx.script("GID_MAPPINGS", ["GID_MAPPING_LIST"],
    lambda mappings: b'"gidMappings": [%s]' % mappings)

ctx.script("UID_MAPPING_LIST", ["ID_MAPPING_ITEM"], lambda mapping: mapping)
ctx.script("GID_MAPPING_LIST", ["ID_MAPPING_ITEM"], lambda mapping: mapping)

ctx.script(
    "ID_MAPPING_ITEM",
    ["CONTAINER_ID", "HOST_ID", "SIZE_VAL"],
    lambda cid, hid, size: b'{"containerID": %s,"hostID": %s,"size": %s}' % (cid, hid, size),
)
ctx.rule("CONTAINER_ID", "0")
ctx.rule("CONTAINER_ID", "1000")
ctx.rule("HOST_ID", "1000")
ctx.rule("HOST_ID", "100000")
ctx.rule("SIZE_VAL", "65536")
ctx.rule("SIZE_VAL", "1")

# =============================================================================
# ROOTFS PROPAGATION / CGROUPS PATH
# =============================================================================

ctx.rule("ROOTFS_PROPAGATION", '"rootfsPropagation": "private"')
ctx.rule("ROOTFS_PROPAGATION", '"rootfsPropagation": "rprivate"')
ctx.rule("ROOTFS_PROPAGATION", '"rootfsPropagation": "shared"')
ctx.rule("ROOTFS_PROPAGATION", '"rootfsPropagation": "rshared"')
ctx.rule("ROOTFS_PROPAGATION", '"rootfsPropagation": "slave"')
ctx.rule("ROOTFS_PROPAGATION", '"rootfsPropagation": "rslave"')

ctx.rule("CGROUPS_PATH", '"cgroupsPath": "/mycontainer"')
ctx.rule("CGROUPS_PATH", '"cgroupsPath": "/docker/containers/123"')
ctx.rule("CGROUPS_PATH", '"cgroupsPath": "/system.slice/myapp.service"')
ctx.rule("CGROUPS_PATH", '"cgroupsPath": "/user.slice/user-1000.slice/session-1.scope"')
ctx.rule("CGROUPS_PATH", '"cgroupsPath": "/machine.slice/libpod-fuzz.scope"')
ctx.rule("CGROUPS_PATH", '"cgroupsPath": "/kubepods/besteffort/pod123/fuzz"')

# =============================================================================
# ANNOTATIONS
# =============================================================================

ctx.rule("ANNOTATIONS", '"annotations": \{\}')
ctx.script("ANNOTATIONS", ["ANNOTATION_ITEM"],
    lambda annotation: b'"annotations": {%s}' % annotation)

ctx.script(
    "ANNOTATION_ITEM",
    ["ANNOTATION_KEY", "ANNOTATION_VALUE"],
    lambda key, value: b'"%s": "%s"' % (key, value),
)

ctx.rule("ANNOTATION_KEY", "com.example.key1")
ctx.rule("ANNOTATION_KEY", "com.example.key2")
ctx.rule("ANNOTATION_KEY", "org.opencontainers.image.created")
ctx.rule("ANNOTATION_KEY", "org.opencontainers.image.authors")
ctx.rule("ANNOTATION_KEY", "io.kubernetes.cri.sandbox-name")
# custom-handler.c trigger: crun's module/wasm delegation system
ctx.rule("ANNOTATION_KEY", "run.oci.handler")
ctx.rule("ANNOTATION_KEY", "module.wasm.image/variant")
ctx.rule("ANNOTATION_KEY", "run.oci.keep_original_groups")

ctx.rule("ANNOTATION_VALUE", "value1")
ctx.rule("ANNOTATION_VALUE", "value2")
ctx.rule("ANNOTATION_VALUE", "2024-01-01T00:00:00Z")
ctx.rule("ANNOTATION_VALUE", "example@example.com")
ctx.rule("ANNOTATION_VALUE", "wasm")
ctx.rule("ANNOTATION_VALUE", "compat-smart")
ctx.rule("ANNOTATION_VALUE", "spin")
ctx.rule("ANNOTATION_VALUE", "true")
ctx.rule("ANNOTATION_VALUE", "false")

# =============================================================================
# Extra fields + extra productions for the high-value structures (more
# productions for an NT = sampled more often).
# =============================================================================

# process.scheduler
ctx.rule("SCHEDULER", '"scheduler": \{"policy": "SCHED_OTHER","nice": 0\}')
ctx.rule("SCHEDULER", '"scheduler": \{"policy": "SCHED_BATCH","nice": 5\}')
ctx.rule("SCHEDULER", '"scheduler": \{"policy": "SCHED_IDLE"\}')
ctx.rule("SCHEDULER", '"scheduler": \{"policy": "SCHED_FIFO","priority": 10\}')
ctx.rule("SCHEDULER", '"scheduler": \{"policy": "SCHED_RR","priority": 50\}')
ctx.rule("SCHEDULER", '"scheduler": \{"policy": "SCHED_DEADLINE","runtime": 30000,"deadline": 100000,"period": 100000\}')

# process.ioPriority
ctx.rule("IO_PRIORITY", '"ioPriority": \{"class": "IOPRIO_CLASS_BE","priority": 4\}')
ctx.rule("IO_PRIORITY", '"ioPriority": \{"class": "IOPRIO_CLASS_RT","priority": 0\}')
ctx.rule("IO_PRIORITY", '"ioPriority": \{"class": "IOPRIO_CLASS_IDLE","priority": 7\}')

# process.oomScoreAdj
ctx.rule("OOM_SCORE_ADJ", '"oomScoreAdj": 0')
ctx.rule("OOM_SCORE_ADJ", '"oomScoreAdj": 100')
ctx.rule("OOM_SCORE_ADJ", '"oomScoreAdj": -500')
ctx.rule("OOM_SCORE_ADJ", '"oomScoreAdj": 1000')
ctx.rule("OOM_SCORE_ADJ", '"oomScoreAdj": -1000')

# process.execCPUAffinity
ctx.rule("EXEC_CPU_AFFINITY", '"execCPUAffinity": \{"initial": "0-1","final": "0"\}')
ctx.rule("EXEC_CPU_AFFINITY", '"execCPUAffinity": \{"final": "0-3"\}')

# process.selinuxLabel / apparmorProfile (shallow without LSMs present)
ctx.rule("SELINUX_LABEL", '"selinuxLabel": "system_u:system_r:container_t:s0:c1,c2"')
ctx.rule("APPARMOR_PROFILE", '"apparmorProfile": "unconfined"')
ctx.rule("APPARMOR_PROFILE", '"apparmorProfile": "docker-default"')

# bundle the extras and hang them off PROCESS
ctx.script("PROCESS_EXTRA", ["SCHEDULER"], lambda x: x)
ctx.script("PROCESS_EXTRA", ["IO_PRIORITY"], lambda x: x)
ctx.script("PROCESS_EXTRA", ["OOM_SCORE_ADJ"], lambda x: x)
ctx.script("PROCESS_EXTRA", ["EXEC_CPU_AFFINITY"], lambda x: x)
ctx.script("PROCESS_EXTRA", ["SELINUX_LABEL"], lambda x: x)
ctx.script("PROCESS_EXTRA", ["APPARMOR_PROFILE"], lambda x: x)
ctx.script("PROCESS_EXTRA", ["SCHEDULER", "OOM_SCORE_ADJ"], lambda a, b: b'%s,%s' % (a, b))
ctx.script("PROCESS_EXTRA", ["IO_PRIORITY", "OOM_SCORE_ADJ"], lambda a, b: b'%s,%s' % (a, b))

ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "PROCESS_EXTRA"],
    lambda terminal, user, args, env, cwd, extra: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s}'
    % (terminal, user, args, env, cwd, extra),
)
ctx.script(
    "PROCESS",
    ["TERMINAL_VAL", "USER_CONFIG", "ARGS_LIST", "ENV_LIST", "CWD_VAL", "CAPABILITIES", "PROCESS_EXTRA"],
    lambda terminal, user, args, env, cwd, caps, extra: b'"process": {"terminal": %s,%s,%s,%s,"cwd": "%s",%s,%s}'
    % (terminal, user, args, env, cwd, caps, extra),
)

# linux.personality
ctx.rule("PERSONALITY", '"personality": \{"domain": "LINUX"\}')
ctx.rule("PERSONALITY", '"personality": \{"domain": "LINUX32"\}')

# linux.resources.unified (cgroup v2 raw)
ctx.rule("UNIFIED", '"unified": \{"memory.high": "134217728"\}')
ctx.rule("UNIFIED", '"unified": \{"memory.max": "268435456","memory.swap.max": "0"\}')
ctx.rule("UNIFIED", '"unified": \{"pids.max": "100"\}')
ctx.rule("UNIFIED", '"unified": \{"cpu.weight": "100"\}')
ctx.rule("UNIFIED", '"unified": \{"cpu.max": "50000 100000"\}')
ctx.rule("UNIFIED", '"unified": \{"io.weight": "default 100"\}')

ctx.script("RESOURCES_UNIFIED", ["UNIFIED"], lambda u: b'"resources": {%s}' % u)
ctx.script("RESOURCES_UNIFIED", ["MEMORY_CONFIG", "UNIFIED"], lambda m, u: b'"resources": {%s,%s}' % (m, u))
ctx.script("RESOURCES_UNIFIED", ["MEMORY_CONFIG", "PIDS_LIMIT", "UNIFIED"], lambda m, p, u: b'"resources": {%s,%s,%s}' % (m, p, u))

# promote UNIFIED into the main RESOURCES NT as well
ctx.script(
    "RESOURCES",
    ["MEMORY_CONFIG", "CPU_LIMIT", "PIDS_LIMIT", "UNIFIED"],
    lambda mem, cpu, pids, uni: b'"resources": {%s,%s,%s,%s}' % (mem, cpu, pids, uni),
)

# namespace set that includes "user" — id-mappings do nothing without a userns
ctx.rule("USERNS_NAMESPACES", '"namespaces": [\{"type": "user"\},\{"type": "mount"\}]')
ctx.rule("USERNS_NAMESPACES", '"namespaces": [\{"type": "user"\},\{"type": "mount"\},\{"type": "pid"\}]')
ctx.rule("USERNS_NAMESPACES", '"namespaces": [\{"type": "user"\},\{"type": "mount"\},\{"type": "pid"\},\{"type": "ipc"\},\{"type": "uts"\}]')

# more LINUX productions so devices / id-mappings / unified show up often
ctx.script("LINUX", ["NAMESPACES", "RESOURCES", "PERSONALITY"],
    lambda ns, res, pers: b'"linux": {%s,%s,%s}' % (ns, res, pers))
ctx.script("LINUX", ["NAMESPACES", "DEVICES", "RESOURCES"],
    lambda ns, dev, res: b'"linux": {%s,%s,%s}' % (ns, dev, res))
ctx.script("LINUX", ["NAMESPACES", "DEVICES", "SECCOMP"],
    lambda ns, dev, sec: b'"linux": {%s,%s,%s}' % (ns, dev, sec))
ctx.script("LINUX", ["NAMESPACES", "RESOURCES_UNIFIED"],
    lambda ns, res: b'"linux": {%s,%s}' % (ns, res))
ctx.script("LINUX", ["NAMESPACES", "RESOURCES_UNIFIED", "CGROUPS_PATH"],
    lambda ns, res, cp: b'"linux": {%s,%s,%s}' % (ns, res, cp))
ctx.script("LINUX", ["USERNS_NAMESPACES", "ID_MAPPINGS"],
    lambda ns, idmap: b'"linux": {%s,%s}' % (ns, idmap))
ctx.script("LINUX", ["USERNS_NAMESPACES", "ID_MAPPINGS", "RESOURCES"],
    lambda ns, idmap, res: b'"linux": {%s,%s,%s}' % (ns, idmap, res))
ctx.script("LINUX", ["USERNS_NAMESPACES", "ID_MAPPINGS", "DEVICES"],
    lambda ns, idmap, dev: b'"linux": {%s,%s,%s}' % (ns, idmap, dev))

# overlay + idmapped mounts + extra options
ctx.rule("MOUNT_TYPE", "overlay")

# overlay (target dirs absent, but exercises the setup path)
ctx.rule("MOUNT_ITEM", '\{"destination": "/merged","type": "overlay","source": "overlay","options": ["lowerdir=/lower","upperdir=/upper","workdir=/work"]\}')
ctx.rule("MOUNT_ITEM", '\{"destination": "/ovl","type": "overlay","source": "overlay","options": ["lowerdir=/a:/b"]\}')

# idmapped bind mounts
ctx.rule("MOUNT_ITEM", '\{"destination": "/idmapped","type": "bind","source": "/tmp/fuzz_bind_src","options": ["bind","rbind","idmap"]\}')
ctx.rule("MOUNT_ITEM", '\{"destination": "/idmapped2","type": "bind","source": "/tmp/fuzz_bind_src","options": ["bind"],"uidMappings": [\{"containerID": 0,"hostID": 1000,"size": 65536\}],"gidMappings": [\{"containerID": 0,"hostID": 1000,"size": 65536\}]\}')

# extra mount options
ctx.rule("MOUNT_OPTION_VAL", "tmpcopyup")
ctx.rule("MOUNT_OPTION_VAL", "copy-symlink")
ctx.rule("MOUNT_OPTION_VAL", "idmap")
ctx.rule("MOUNT_OPTION_VAL", "ridmap")
ctx.rule("MOUNT_OPTION_VAL", "src-nofollow")
ctx.rule("MOUNT_OPTION_VAL", "dest-nofollow")

# =============================================================================
# Known-good complete configs, to anchor crun's success path in the corpus.
# args/caps stay as sub-trees so they remain mutable; root.path is rewritten to
# the FUSE mount by the harness.
# =============================================================================

# rich: caps, bind mount, device, masked/readonly paths, resources
ctx.script(
    "OCI_CONFIG",
    ["ARGS_LIST", "CAPABILITIES"],
    lambda args, caps: b'{"ociVersion": "1.0.0","process": {"terminal": false,"user": {"uid": 0,"gid": 0},%s,"env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin","TERM=xterm"],"cwd": "/",%s,"noNewPrivileges": true,"oomScoreAdj": 100},"root": {"path": "rootfs","readonly": false},"hostname": "fuzz","mounts": [{"destination": "/proc","type": "proc","source": "proc"},{"destination": "/dev","type": "tmpfs","source": "tmpfs","options": ["nosuid","strictatime","mode=755","size=65536k"]},{"destination": "/sys","type": "sysfs","source": "sysfs","options": ["nosuid","noexec","nodev","ro"]},{"destination": "/tmp","type": "tmpfs","source": "tmpfs"},{"destination": "/mnt","type": "bind","source": "/tmp/fuzz_bind_src","options": ["rbind","ro"]}],"linux": {"namespaces": [{"type": "pid"},{"type": "mount"},{"type": "ipc"},{"type": "uts"}],"resources": {"memory": {"limit": 134217728},"pids": {"limit": 1024}},"devices": [{"path": "/dev/null","type": "c","major": 1,"minor": 3}],"maskedPaths": ["/proc/kcore","/proc/timer_list"],"readonlyPaths": ["/proc/sysrq-trigger"]},"annotations": {}}'
    % (args, caps),
)

# minimal golden: smallest config that still runs to completion
ctx.script(
    "OCI_CONFIG",
    ["ARGS_LIST"],
    lambda args: b'{"ociVersion": "1.0.0","process": {"terminal": false,"user": {"uid": 0,"gid": 0},%s,"env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],"cwd": "/"},"root": {"path": "rootfs","readonly": false},"hostname": "fuzz","mounts": [{"destination": "/proc","type": "proc","source": "proc"},{"destination": "/dev","type": "tmpfs","source": "tmpfs"},{"destination": "/sys","type": "sysfs","source": "sysfs","options": ["ro"]}],"linux": {"namespaces": [{"type": "pid"},{"type": "mount"}],"resources": {"pids": {"limit": 1024}}},"annotations": {}}'
    % args,
)
