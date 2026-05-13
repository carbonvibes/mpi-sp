Flavio Toffalini: I think the crun fuzzer creates a new cgroup for every inputFlavio Toffalini: and the kernel has a max number of cgroups that can generateArjun Sekar: yeah that makes senseFlavio Toffalini: this also means that after a while, your campaigns are unable to load new containersFlavio Toffalini: and I can't run new containers either :DFlavio Toffalini: ok now it worksFlavio Toffalini: I did a workaroundFlavio Toffalini: but the problem should be solved on your sideFlavio Toffalini: because I am sure that after a while your harness can't generate new containers eitherArjun Sekar: Flavio Toffalini said:

ok now it works

I actually killed my fuzzing campaigns this morning so it should technically work igArjun Sekar: Flavio Toffalini said:

but the problem should be solved on your side

yeah I'll look into it and will do something about it like maybe cap the generations...Flavio Toffalini: mmmh I am not sure how to solve the problemFlavio Toffalini: there we may need help from @Moritz Sanft when he has time :DFlavio Toffalini: the most likely problem is: you keep creating new cgroups, but you do not remove then properly from the systemFlavio Toffalini: they behave like "stopped containers", but without a proper IDFlavio Toffalini: for instance, claude suggests this command :D 
# Find cgroups with no live PIDs (candidates for cleanup)
find /sys/fs/cgroup -name "cgroup.procs" | while read f; do
    [ -z "$(cat "$f" 2>/dev/null)" ] && echo "${f%/cgroup.procs}"
done
Flavio Toffalini: it shows all the cgroup that are deadFlavio Toffalini: but not removed from the kernelFlavio Toffalini: LoLArjun Sekar: Flavio Toffalini said:

for instance, claude suggests this command :D 
# Find cgroups with no live PIDs (candidates for cleanup)
find /sys/fs/cgroup -name "cgroup.procs" | while read f; do
    [ -z "$(cat "$f" 2>/dev/null)" ] && echo "${f%/cgroup.procs}"
done



I just ran this command... omg lolFlavio Toffalini: for some reason, either your haness or Morit's one does not clean up the systemFlavio Toffalini: try to work on thatFlavio Toffalini: and I am sure this causes your fuzzer to stop working after a while. Probably you can also check for "system" errors and figure out if at some point yuoi simply do not create new containersFlavio Toffalini: that also justifies the plateau

Flavio Toffalini: yah, spend some time in solving this issue. We will probably need campaigns running for >24h. So cleaning the state is important

Moritz Sanft: Thanks for finding out. I don't know if I had higher limits in my testing setup by chance. Iirc the harness tries to stop and remove the container afterwards. It might not be the case or I might do it wrong. We definitely to investigate. Most likely, this just didn't become a problem in my testing as the longest campaign was ~20h. Plateauing at 12% is weird, since I'm quite certain I was at something around 26% back then. But we should probably solve the cgroup and mount problems first (i.e. just make sure to really clean up the state for the container probably, crun has utilities for that); Pointing Claude at it should be enough, I think. This could then be verified with a single manual invocation and checking the state before and afterwards. @Arjun Sekar, can you do this by any chance? If you can, keep the modified harness patch around, it would also be useful for SemSan, I think.Moritz Sanft: And please expect longer response times from me than usual this week since I'm travelling a lot. I'll try to come in every morning though