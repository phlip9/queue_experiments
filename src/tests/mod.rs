cfg_loom! {
    mod fuzz_arc_cell;
    mod fuzz_bilock;
    mod fuzz_spsc_bilock;
    mod fuzz_spsc_lock;
}

cfg_not_loom! {
    mod bilock;
    mod spsc_bilock;
    mod spsc_lock;
}
