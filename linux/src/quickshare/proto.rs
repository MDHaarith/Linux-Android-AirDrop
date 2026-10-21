//! Nearby Connections / Nearby Sharing protobufs, generated from
//! `proto/quickshare` by `tools/protogen`. Module nesting mirrors the proto
//! packages because the generated code refers across them with `super::`.

#![allow(clippy::all, clippy::pedantic, dead_code, deprecated, missing_docs)]

pub mod securemessage {
    include!("proto/securemessage.rs");
}

pub mod securegcm {
    include!("proto/securegcm.rs");
}

pub mod location {
    pub mod nearby {
        pub mod connections {
            include!("proto/location.nearby.connections.rs");
        }
        pub mod proto {
            pub mod sharing {
                include!("proto/location.nearby.proto.sharing.rs");
            }
        }
    }
}

pub mod sharing {
    pub mod nearby {
        include!("proto/sharing.nearby.rs");
    }
}
