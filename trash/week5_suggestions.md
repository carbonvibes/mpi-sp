Things That Are Genuinely Missing or Problematic
1. AddFileOp never generates UpdateFile on baseline files
This is the biggest functional gap. AddFileOp only generates CreateFile or Mkdir at random paths. But the most important mutation for an OCI runtime fuzzer is updating content of files that already exist in the baseline — /etc/passwd, /etc/hostname, config files the runtime definitely reads. Those files exist in the baseline and the target will read them every single run. Mutating their content is high-value. Right now no mutator stage targets them. You need either a dedicated UpdateBaselineFile stage or to extend AddFileOp to also emit UpdateFile ops on paths drawn from the known baseline.
2. AddFileOp and DestructiveMutator don't know what actually exists
Both stages generate paths from PATH_COMPONENTS without knowing what directories exist in the baseline VFS. cp_ensure_parents saves you from hard failures on creates, but DestructiveMutator will generate DeleteFile and Rmdir on random paths that almost certainly don't exist — which explains the 84/100 partial count. The paths the target actually cares about (real baseline paths) are never targeted by destructive ops. Both stages need access to a list of paths that actually exist in the baseline so they can generate ops that have a real chance of doing something meaningful.
3. PATH_COMPONENTS vocabulary is too small and shallow for real OCI rootfs
Currently paths go 1–3 components deep from a 15-word vocabulary. A real container rootfs has paths like /usr/lib/x86_64-linux-gnu/libm.so.6, /etc/ld.so.cache, /run/containerd/io.containerd/.... When you integrate a real rootfs in a later week, the mutator will keep generating paths that don't exist and almost never hit real interesting locations. This vocabulary needs to be derived from the actual baseline tree, not hardcoded.
4. SpliceDelta is using a hardcoded pool
The Phase A SpliceDelta draws from initial_corpus_pool() — 3 hardcoded deltas. This is acknowledged as a placeholder but it means SpliceDelta is essentially deterministic right now. When Phase B wires up the real LibAFL corpus, SpliceDelta needs to be redesigned to receive the live corpus through LibAFL's state mechanism, not a static pool stored in the mutator struct. The interface needs to change, not just the data.
5. Checksum ordering is insertion-order dependent
The kaching.md already flags this as a known limitation — "two VFS instances with the same files created in different orders produce different checksums." This becomes a real correctness problem in Phase B when corpus management starts relying on checksums to deduplicate or identify inputs. Two deltas that produce the same final VFS state but in different op orders will look like different inputs. This needs to be fixed — alphabetical sort of paths before hashing — before the corpus grows large enough to pollute itself with duplicates.
6. The dumb loop always starts from the same seed delta
Every iteration clones CreateFile("/input", b"seed") and then mutates it once. There's no corpus accumulation — interesting mutations are never fed back as starting points for future mutations. So SpliceDelta in particular has nothing real to splice from yet, and ByteFlipFileContent is always starting from the same 4-byte seed content. The 39% semantic yield number is therefore an underestimate of what you'd get with real corpus feedback, but it also means you're not really testing the mutators in a realistic scenario.

Smaller Things Worth Noting
MutatePath replaces a component but never changes path depth. A path that starts at depth 2 stays at depth 2. You'd also want mutations that go deeper or shallower, since path depth affects directory traversal behavior in the target.
DestructiveMutator generates timestamps in a very narrow range — random UNIX timestamps could mean anything. Worth making sure they include edge cases: zero, negative (pre-epoch), far future, values around the current time. Timestamp handling bugs in OCI runtimes are a real class of bugs.
validate_delta is debug-only. That's the right call for performance, but it means in release builds a malformed delta from a future mutator bug would silently reach the C layer. Consider logging (not asserting) malformed deltas in release builds at least during the development phase.

What To Fix Before Phase B
In priority order:

Add baseline path enumeration — enumerate all paths in the baseline VFS at startup and pass them to AddFileOp, DestructiveMutator, and MutatePath so they can target real paths. This is a single Vec<String> computed once.
Add UpdateFile on baseline paths — either extend AddFileOp or add a dedicated stage. This is the highest-value mutation you're currently missing.
Fix checksum ordering — sort paths alphabetically before hashing. Easy fix, high correctness impact.
Redesign SpliceDelta's corpus access — decide now how it will access the live LibAFL corpus in Phase B, so you don't have to restructure the mutator interface mid-phase.

Everything else can wait. The core loop is sound.