Linux namespaces

A namespace is a kernel mechanism that gives a process an isolated view of a system resource. There are several types:

mount namespace (CLONE_NEWNS): the process sees its own filesystem tree. Mounts created inside it don't affect the rest of the system.
user namespace (CLONE_NEWUSER): the process has its own UID/GID mapping. A process can be root (UID 0) inside the namespace but map to an unprivileged UID on the host.
pid namespace: its own process ID space.
network namespace: its own network interfaces, routing tables.
When you call unshare(CLONE_NEWNS), the kernel gives the calling process a private copy of the current mount table. After that, any mount() or umount() calls only affect that process and its children — the host sees nothing.

What crun does with namespaces

crun reads the OCI config JSON. The linux.namespaces array tells it what isolation to set up. For each entry, crun calls unshare() or clone() with the corresponding flag.

The sequence inside crun for a container run:

Fork a child process (the container init)
Child calls unshare() for each namespace in the config
If user namespace: write UID/GID mappings to /proc/<child_pid>/uid_map and gid_map
If mount namespace: set up rootfs mounts, bind mounts, proc, sysfs, devtmpfs inside the new namespace
Do pivot_root — change the child's filesystem root to the container rootfs (our FUSE mount)
After pivot_root: exec /bin/bash
The parent (crun harness) and child communicate via a "sync socket" throughout this process. At each step, the child writes a message saying "step N done". If the child dies mid-way without writing, the parent reads EOF on the sync socket.

What happens with NO mount namespace

If the config has no mount namespace entry (linux.namespaces is empty or missing), crun skips step 4's unshare(CLONE_NEWNS). The child stays in the same mount namespace as the fuzzer process.

Now the problem. The FUSE VFS has our rootfs at /tmp/campaign3-fuse-<pid>/. The FsDelta mutator can create symlinks like:


/tmp → ../../../../tmp
This is stored in the VFS as a symlink node. When the kernel accesses /tmp/campaign3-fuse-<pid>/tmp through FUSE, it gets the symlink target ../../../../tmp, resolves it relative to the FUSE mountpoint directory, and ends up at the real host /tmp.

If the OCI config also has a mount entry like {"type":"sysfs","destination":"/tmp"}, crun (in the child, which shares the fuzzer's mount namespace) calls:


mount("sysfs", "/tmp/campaign3-fuse-<pid>/tmp", "sysfs", ...)
The kernel follows the FUSE symlink escape → mounts sysfs at host /tmp. This shadows /tmp/campaign3-fuse-<pid>/ entirely — the FUSE mount is now unreachable. The FUSE thread in the fuzzer process gets no more requests. The next crun execution tries to access the rootfs and hangs. LibAFL times out → panics → fuzzer process dies.

The fix I added

In override_rootfs_path (called for every input before it reaches crun), I inject {"type":"mount"} into the namespaces array if it's not already there.


if !has_mount {
    arr.push(serde_json::json!({"type": "mount"}));
}
Now every config crun receives always has a mount namespace. The child always calls unshare(CLONE_NEWNS) first. Any mounts it makes — including a sysfs mount at host /tmp via symlink escape — happen inside the child's private mount namespace. When the child exits, the kernel destroys that namespace and all its mounts. The fuzzer's namespace is completely untouched. Fuzzer never dies from this.

What went wrong

The previous ASAN run (18hrs, 0 crashes) ran WITHOUT this fix. The grammar sometimes generated configs with only ["user"] namespace. crun got ["user"] → user namespace setup → failed with a proper error code (e.g. EPERM) written to the sync socket → parent read the error with err->status != 0 → child_crashed = false → harness did continue → no crash recorded. Normal non-crash error handling.

After the fix, grammar generates ["user"] → we inject ["user", "mount"] → crun now sets up BOTH user AND mount namespace together. This is a different code path. Something in that combined setup (user namespace UID mapping + mount namespace setup together) causes the child to die by signal — SIGSEGV or SIGABRT — before it can write anything to the sync socket. Parent reads EOF with err->status == 0 → child_crashed = true → raise(SIGSEGV) → AFL++ records a crash. This happened 273 times in 4 hours.

So the fix solved the fuzzer-dying problem, but by forcing crun into a code path (user+mount namespace combined) it never hit before, we accidentally triggered a flood of crashes from what appears to be a real bug in crun's combined namespace setup.

Are these real bugs?

Yes — crun should not SIGSEGV just because a config requests both user and mount namespaces. That is a valid OCI configuration. The crashes are real child process deaths by signal, not false positives from our detection logic. But they're all hitting the same code path, so it's one bug (or a small number of related bugs) triggering 273 times, not 273 independent bugs.

launch_campaigns.sh
The kernel follows the FUSE symlink escape → mounts sysfs at host /tmp. This shadows /tmp/campaign3-fuse-<pid>/ entirely — the FUSE mount is now unreachable. The FUSE thread in the fuzzer process gets no more requests. The next crun execution tries to access the rootfs and hangs. LibAFL times out → panics → fuzzer process dies.

this is just a theory might not be the actual case... but u have some doubts in your theory can you explain this in detail??? i undertand that now that it ends up at host /tmp but after that i dont understand how it dies give me step by step detailed explanation and also tell me what happens if it did have mount namespace then what happens in this exact scenario
Let me be precise and honest about what I know and what I'm uncertain about.

The cast of processes:


fuzz_combined_afl (LibAFL fuzzer)        ← PID = F
  └─ crun harness (forkserver)            ← PID = H, has __AFL_LOOP(1)
       └─ crun harness child              ← forked per test case by H
            └─ container init             ← forked by libcrun_container_run()
All four share the same mount namespace (created by sudo unshare -m fuzz_combined_afl), unless the container init calls unshare(CLONE_NEWNS).

Step by step — NO mount namespace:

Step 1. LibAFL tells forkserver H: "run this input." H forks a child. Child calls libcrun_container_run(). libcrun forks the container init.

Step 2. Container init has NO mount namespace in its config → it skips unshare(CLONE_NEWNS). It is now in the same mount namespace as F, H, and the child.

Step 3. Config has mounts: [{"destination":"/tmp","source":"sysfs"}]. Container init calls:


mount("sysfs", "/tmp/campaign3-fuse-F/tmp", "sysfs", ...)
The kernel resolves /tmp/campaign3-fuse-F/tmp through FUSE. FUSE returns the symlink target ../../../../tmp. Kernel follows it → resolves to /tmp on the host.

Step 4. mount("sysfs", "/tmp", "sysfs", ...) succeeds. Sysfs is now mounted at /tmp in the shared namespace. Every process in this namespace — F, H, child, container init — now sees sysfs when they open /tmp. The directory /tmp/campaign3-fuse-F/ is a path under /tmp, so it is shadowed. You cannot reach it by path anymore.

Step 5. Container init continues setup. It eventually tries to access the rootfs at /tmp/campaign3-fuse-F/ (that's what root.path says). The kernel looks for campaign3-fuse-F under /tmp → finds sysfs entries instead → ENOENT. libcrun_set_mounts or pivot_root fails.

Step 6 — here is where I'm uncertain. One of two things happens:

libcrun returns a clean error through the sync socket (err->status != 0) → harness sees ret < 0, child_crashed = false, does continue. Container init exits. Forkserver H is still alive. LibAFL continues.

OR the container init blocks in D state trying to access FUSE (because it sent a FUSE request before the shadow happened and is waiting for a reply that never comes). H is waiting on waitpid(). LibAFL's timeout fires → sends SIGKILL to container init. Init dies. H reads SIGKILL status, tries to write it to LibAFL's pipe. If LibAFL already closed or reset the pipe waiting for the response, H gets SIGPIPE → H dies. LibAFL then reports "Unable to communicate with fork server."

I can't prove which path actually happens without attaching a debugger to the live process. The second path is my best guess for why H dies, but it's not proven.

Step 7. Either way, after step 4, every future crun run is broken. The FUSE mount is shadowed for all subsequent iterations. The FUSE thread in F is blocked in fuse_loop() waiting for requests that will never come (path is inaccessible). Each new crun child tries to access /tmp/campaign3-fuse-F/ → ENOENT → fails. Eventually something in this chain kills H (the forkserver), and F panics with "Unable to communicate with fork server."

Same scenario WITH mount namespace:

Step 1 is identical. But now the config has ["mount"] injected.

Step 2. Container init calls unshare(CLONE_NEWNS). The kernel gives it a private copy of the current mount table. The container init is now in its own namespace. F, H, and the child are still in the original shared namespace.

Step 3-4 identical inside the container. mount("sysfs", "/tmp", "sysfs", ...) is called. But this mount happens only in the container init's private namespace. F, H, and the child see nothing change. Their /tmp is untouched.

Step 5. Container init exits (or crashes). The kernel destroys the private namespace and all its mounts along with it. Sysfs at /tmp vanishes.

Step 6. The FUSE mount at /tmp/campaign3-fuse-F/ is still perfectly accessible. The FUSE thread is still serving requests. The next iteration runs normally.

The honest summary of what I'm sure about vs not:

Sure: sysfs ends up at host /tmp without mount namespace, shadowing the FUSE mountpoint. With mount namespace it stays contained.

Not sure: the exact chain of events between "FUSE mountpoint shadowed" and "forkserver reports Unable to communicate." It likely involves either D-state blocking + timeout + SIGPIPE, or some other pipe/signal interaction. The behavior is consistent with the theory but I haven't proven the exact kill mechanism.

i have some questions 
1) you said Container init calls:


mount("sysfs", "/tmp/campaign3-fuse-F/tmp", "sysfs", ...)

how does the container init knows to use the fuse mount like is that logic incorporated??

2) whats is sysfs and im confused about step 5 like is sysfs like rootfs and if the kernel looks for campaign3-fuse-F under /tmp then it will find it right why care about sysfs??
Good questions. Let me answer both clearly.

Question 1: How does container init know to use the FUSE mount path?

It doesn't explicitly know about FUSE at all. Here's the chain:

Our override_rootfs_path function modifies the config JSON before crun sees it, changing root.path from "rootfs" to "/tmp/campaign3-fuse-F". So crun reads the config and sees:


"root": {"path": "/tmp/campaign3-fuse-F"}
When crun processes the mounts array, for each entry it constructs the actual host path as:


full_destination = rootfs_path + mount.destination
                 = /tmp/campaign3-fuse-F + /tmp
                 = /tmp/campaign3-fuse-F/tmp
crun then calls mount("sysfs", "/tmp/campaign3-fuse-F/tmp", ...). crun has no idea that path goes through FUSE — it just passes the string to the kernel. The kernel resolves the path, hits the FUSE mount, calls fvfs_readlink("/tmp") on our FUSE server, gets ../../../../tmp back, follows it, ends up at host /tmp. The FUSE magic is completely transparent to crun.

Question 2: What is sysfs and why can't the kernel find campaign3-fuse-F under /tmp?

sysfs is a virtual filesystem the kernel provides at /sys — it exposes kernel internals: devices, cgroups, power management etc. It has nothing to do with storage. Not important what it contains here.

Your confusion about step 5 is actually the key thing to understand. This is about how mounting works in Linux.

Before the sysfs mount, /tmp looks like this:


/tmp/                    ← tmpfs
    campaign3-fuse-F/    ← our FUSE mountpoint (a directory in tmpfs)
    c3_0/
    c3_1/
    ...
When mount("sysfs", "/tmp", ...) runs, the kernel does NOT modify the tmpfs underneath. It stacks sysfs on top of /tmp. Think of it like placing a new layer completely over the existing one. Now when anything accesses /tmp, the kernel serves the sysfs root — not the old tmpfs. The original contents (campaign3-fuse-F/, c3_0/, etc.) are still physically there on the tmpfs but they are completely hidden. No process can reach them via the path /tmp/anything because the kernel never gets past the sysfs layer to look underneath.

So when the kernel tries to resolve /tmp/campaign3-fuse-F/:

Opens /tmp → sees sysfs root
Looks for entry named campaign3-fuse-F inside sysfs → doesn't exist
Returns ENOENT
The directory is there underneath — it's just unreachable. Like putting a solid metal plate over a trapdoor. The room below still exists but you can't get to it.

if we see into the sync socket then we can understand what the error is actually right?? i mean is that why we need asan?
No — the sync socket can't give you that information. Here's why:

The sync socket is a communication channel between the container init (child) and the crun parent. The child writes structured messages to it as it progresses through setup — "user namespace done", "mounts done", etc. The parent reads these to know where the child is.

When the child dies by signal (SIGSEGV, SIGABRT), it dies instantly. It can't write anything. The parent reads EOF with errno=0 — that's it. The sync socket just tells you "the child died before finishing step N." It tells you nothing about WHERE in the code it crashed or WHY.

So what the sync socket actually tells us:


Child died before sending anything  →  "read from sync socket" + status=0
Child died during exec setup        →  "read from exec fifo"  + status=0
Child died with a clean error       →  err->status = errno, err->msg = description
Only the third case has useful information. The first two (which are what triggers our child_crashed detection) have nothing useful — just that the child is dead.

That's exactly why ASAN helps. Without ASAN:

Child SIGSEGVs somewhere in crun's setup code
Sync socket gets EOF
You know: crash happened. You don't know: where, what type, what was corrupted.
With ASAN:

Child hits a memory error
ASAN intercepts it BEFORE the SIGSEGV, prints the full stack trace, error type, allocation site
Then calls abort()
Sync socket still gets EOF, but now you have a report telling you exactly what went wrong
So ASAN is the right tool here. The sync socket is just a "something went wrong" signal — ASAN is what tells you the actual story.

last doubt im currently running 6 instaces of c3 fuzzing but when is tried to look into /tmp/camapaign3 its just empty is that because it does actively have files in it but its in vfs am i right is that why it appears empty?
Yes, exactly right. The directory /tmp/campaign3-fuse-<pid>/ is just a mount anchor — it's an empty directory on the real filesystem (tmpfs) that FUSE uses as its attachment point. The actual files and directories you see when you access it come entirely from the in-memory VFS served by our FUSE thread.

When you do ls /tmp/campaign3-fuse-<pid>/:

The kernel sees the path hits a FUSE mount
Sends a readdir request to our fvfs_readdir function via /dev/fuse
fvfs_readdir reads from the in-memory VFS node tree
Returns /bin, /etc, /proc, /dev, etc. — all from VFS memory
You see those entries — none of them exist as real files on disk
If you unmount FUSE and look at the underlying directory — it's completely empty. Always has been. It's just a hook in the filesystem tree that FUSE intercepts.

That's also why when the fuzzer dies without cleanup, all you're left with is the empty directory (what we cleaned up earlier). The FUSE mount disappears when the process dies (kernel automatically closes /dev/fuse), and the bare empty mountpoint directory is all that remains on disk.

And yes, every iteration the VFS state changes (via vfs_reset_to_snapshot then apply_delta) but none of that touches the real filesystem at all — it's all in memory, served on demand by the FUSE thread.