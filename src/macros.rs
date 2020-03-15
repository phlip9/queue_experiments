macro_rules! cfg_loom {
    ($($item:item)*) => {
        $( #[cfg(loom)] $item )*
    }
}

macro_rules! cfg_not_loom {
    ($($item:item)*) => {
        $( #[cfg(not(loom))] $item )*
    }
}
