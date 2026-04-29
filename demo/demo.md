cd /home/arjun/mpi-sp/mutator
rm -rf corpus_foobar solutions_foobar
cargo run --release --bin fuzz_libafl -- foobar 2>&1 | tee /tmp/foobar_fuzz.log


python3 /home/arjun/mpi-sp/fuzz_dashboard/server.py /tmp/foobar_fuzz.log foobar


cd /home/arjun/mpi-sp/mutator && cargo build --release 2>&1 


cargo run --release --bin fuzz_foobar 2>&1 | tee /tmp/foobar_fuzz.log
cargo run --release --bin fuzz_foobar_cmplog 2>&1 | tee /tmp/foobar_fuzz.log

// Line ~655-656: uncomment
let i2s_scheduled = HavocScheduledMutator::new(tuple_list!(FsDeltaI2SMutator::new()));
let i2s_stage     = StdMutationalStage::new(i2s_scheduled);
// Line ~676: change to
let mut stages = tuple_list!(i2s_stage, havoc_stage);
// Line ~769: change to
tuple_list!(edges_observer, cmplog_observer),

