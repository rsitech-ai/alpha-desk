mod index;
mod vector;

pub use index::{ExactVectorIndex, VectorIndex};
pub use vector::{
    AnalogueMatch, AnalogueSet, MemoryEntry, MemoryQuery, MemorySupport, VECTOR_DIMENSION_COUNT,
    VectorManifest, contributing_dimensions, squared_distance,
};
