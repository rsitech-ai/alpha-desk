use canonical_events::{ConfirmationClass, EventKind};
use domain_types::{BlockHeight, ChainId};
use hl_capture::synthetic_fixture_block;

#[test]
fn synthetic_fixture_blocks_are_deterministic_contiguous_and_explicitly_committed() {
    let chain = ChainId::new("fixture-mainnet").expect("chain");
    let first = synthetic_fixture_block(&chain, BlockHeight::new(10)).expect("first fixture block");
    let repeated =
        synthetic_fixture_block(&chain, BlockHeight::new(10)).expect("repeated fixture block");
    let next = synthetic_fixture_block(&chain, BlockHeight::new(11)).expect("next fixture block");

    assert_eq!(first, repeated);
    assert_ne!(first.canonical_block_hash(), next.canonical_block_hash());
    assert_eq!(first.chain_id(), &chain);
    assert_eq!(first.block_height(), BlockHeight::new(10));
    assert_eq!(
        first.confirmation_class(),
        ConfirmationClass::CommittedPrimary
    );
    assert_eq!(first.events().len(), 1);
    assert_eq!(first.events()[0].payload().kind(), EventKind::TradeMatched);
    assert_eq!(
        first.events()[0].source_evidence()[0].source_id().as_str(),
        "synthetic-fixture"
    );
}
