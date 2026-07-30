use canonical_ledger::{
    EpisodeAttributionResolutionV1, EpisodeCloseCauseV1, EpisodeCompletenessV1,
    EpisodeEffectKindV1, EpisodeStatusV1, PositionEpisodeCurrentRecordV1,
    PositionEpisodeEffectFactRecordV1, PositionEpisodeRecordV1, PositionStateError,
    derive_position_episode_id,
};
use domain_types::{Address, BlockHeight, EventId, MarketId, PositionEpisodeId};

const ACCOUNT: &str = "0x1111111111111111111111111111111111111111";
const OTHER_ACCOUNT: &str = "0x2222222222222222222222222222222222222222";
const MARKET: &str = "perp:BTC";
const OPEN_EVENT: &str = "evt-open-fixed";
const EPISODE_ID: &str = "pos_ep_b3560541bc26a1624a0844b5a808325b78a4d2cdec3d81a2e4355ef1c974596d";
const RULE_VERSION: &str = "hyperliquid-alpha-desk-canonical-position-episode@1.0.0";
const EPISODE_KEY_GOLDEN: &[u8] =
    b"\x00\x00\x00\x00\x00\x00\x00\x47pos_ep_b3560541bc26a1624a0844b5a808325b78a4d2cdec3d81a2e4355ef1c974596d";
const CURRENT_KEY_GOLDEN: &[u8] = b"\x00\x00\x00\x00\x00\x00\x00\x14\
    \x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\
    \x00\x00\x00\x00\x00\x00\x00\x08perp:BTC";
const EFFECT_KEY_GOLDEN: &[u8] = b"\x00\x00\x00\x00\x00\x00\x00\x0eevt-open-fixed\
    \x00\x00\x00\x00\x00\x00\x00\x14\
    \x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\x11\
    \x00\x00\x00\x00\x00\x00\x00\x08perp:BTC\
    \x00\x00\x00\x00\x00\x00\x00\x01\x00";

fn account() -> Address {
    Address::parse_api(ACCOUNT).unwrap()
}

fn market() -> MarketId {
    MarketId::new(MARKET).unwrap()
}

fn opening_event() -> EventId {
    EventId::new(OPEN_EVENT).unwrap()
}

fn episode_id() -> PositionEpisodeId {
    PositionEpisodeId::new(EPISODE_ID).unwrap()
}

fn open_episode_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/position-episode/v1","#,
            r#""episode_id":"{EPISODE_ID}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""market_id":"{MARKET}","#,
            r#""opening_anchor_event_id":"{OPEN_EVENT}","#,
            r#""opening_leg_ordinal":0,"#,
            r#""opening_position":"0.00000000","#,
            r#""close_event_id":null,"#,
            r#""close_cause":null,"#,
            r#""completeness":"complete_from_flat","#,
            r#""buy_quantity":"0.25000000","#,
            r#""buy_notional":"16250","#,
            r#""sell_quantity":"0.00000000","#,
            r#""sell_notional":"0","#,
            r#""funding_paid":"0.000000","#,
            r#""funding_received":"0.000000","#,
            r#""status":"open","#,
            r#""last_event_id":"{OPEN_EVENT}","#,
            r#""last_block_height":1600}}"#
        ),
        EPISODE_ID = EPISODE_ID,
        ACCOUNT = ACCOUNT,
        MARKET = MARKET,
        OPEN_EVENT = OPEN_EVENT,
    )
    .into_bytes()
}

fn current_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/position-episode-current/v1","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""market_id":"{MARKET}","#,
            r#""episode_id":"{EPISODE_ID}","#,
            r#""attribution_resolution":"resolved","#,
            r#""last_event_id":"{OPEN_EVENT}","#,
            r#""last_block_height":1600}}"#
        ),
        ACCOUNT = ACCOUNT,
        MARKET = MARKET,
        EPISODE_ID = EPISODE_ID,
        OPEN_EVENT = OPEN_EVENT,
    )
    .into_bytes()
}

fn opened_effect_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/position-episode-effect-fact/v1","#,
            r#""event_id":"{OPEN_EVENT}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""market_id":"{MARKET}","#,
            r#""leg_ordinal":0,"#,
            r#""episode_id":"{EPISODE_ID}","#,
            r#""effect_kind":"opened","#,
            r#""buy_quantity_delta":"0.25000000","#,
            r#""buy_notional_delta":"16250","#,
            r#""sell_quantity_delta":"0.00000000","#,
            r#""sell_notional_delta":"0","#,
            r#""funding_paid_delta":"0.000000","#,
            r#""funding_received_delta":"0.000000","#,
            r#""close_cause":null,"#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        OPEN_EVENT = OPEN_EVENT,
        ACCOUNT = ACCOUNT,
        MARKET = MARKET,
        EPISODE_ID = EPISODE_ID,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

#[test]
fn episode_id_derivation_is_literal_bounded_and_input_separating() {
    let derived = derive_position_episode_id(&account(), &market(), &opening_event(), 0).unwrap();
    assert_eq!(derived.as_str(), EPISODE_ID);
    assert_eq!(derived.as_str().len(), "pos_ep_".len() + 64);
    assert!(
        derived
            .as_str()
            .bytes()
            .skip("pos_ep_".len())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );

    let ordinal_one =
        derive_position_episode_id(&account(), &market(), &opening_event(), 1).unwrap();
    assert_eq!(
        ordinal_one.as_str(),
        "pos_ep_3ab110b14be98823752ad96a7f4d4c2eb52649fe30e206b4c82399a79ebdb14e"
    );
    assert_ne!(ordinal_one, derived);
    assert_ne!(
        derive_position_episode_id(
            &Address::parse_api(OTHER_ACCOUNT).unwrap(),
            &market(),
            &opening_event(),
            0,
        )
        .unwrap(),
        derived
    );
    assert_ne!(
        derive_position_episode_id(
            &account(),
            &MarketId::new("perp:ETH").unwrap(),
            &opening_event(),
            0,
        )
        .unwrap(),
        derived
    );
    assert_ne!(
        derive_position_episode_id(
            &account(),
            &market(),
            &EventId::new("evt-open-other").unwrap(),
            0,
        )
        .unwrap(),
        derived
    );
    assert_eq!(
        derive_position_episode_id(&account(), &market(), &opening_event(), 2),
        Err(PositionStateError::InvalidRecord)
    );
}

#[test]
fn episode_record_freezes_literal_wire_key_and_accessors() {
    let bytes = open_episode_bytes();
    let record = PositionEpisodeRecordV1::decode(&bytes).unwrap();
    let key = PositionEpisodeRecordV1::state_key(&episode_id()).unwrap();
    assert_eq!(key.namespace(), "position-episode.v1");
    assert_eq!(key.key(), EPISODE_KEY_GOLDEN);
    assert_eq!(key.key().len(), 79);
    for malformed in [
        PositionEpisodeId::new(format!("pos_ep_{}", "a".repeat(63))).unwrap(),
        PositionEpisodeId::new(format!("pos_ep_{}", "A".repeat(64))).unwrap(),
        PositionEpisodeId::new("episode-not-derived").unwrap(),
    ] {
        assert_eq!(
            PositionEpisodeRecordV1::state_key(&malformed),
            Err(PositionStateError::InvalidRecord)
        );
    }
    assert_eq!(
        PositionEpisodeRecordV1::decode_at(&key, &bytes).unwrap(),
        record
    );
    assert_eq!(record.episode_id(), &episode_id());
    assert_eq!(record.account_id(), account());
    assert_eq!(record.market_id(), &market());
    assert_eq!(record.opening_anchor_event_id(), &opening_event());
    assert_eq!(record.opening_leg_ordinal(), 0);
    assert_eq!(record.opening_position().to_string(), "0.00000000");
    assert_eq!(record.close_event_id(), None);
    assert_eq!(record.close_cause(), None);
    assert_eq!(
        record.completeness(),
        EpisodeCompletenessV1::CompleteFromFlat
    );
    assert_eq!(record.buy_quantity().to_string(), "0.25000000");
    assert_eq!(record.buy_notional().to_string(), "16250");
    assert_eq!(record.sell_quantity().to_string(), "0.00000000");
    assert_eq!(record.sell_notional().to_string(), "0");
    assert_eq!(record.funding_paid().to_string(), "0.000000");
    assert_eq!(record.funding_received().to_string(), "0.000000");
    assert_eq!(record.status(), EpisodeStatusV1::Open);
    assert_eq!(record.last_event_id(), &opening_event());
    assert_eq!(record.last_block_height(), BlockHeight::new(1600));
}

#[test]
fn current_record_freezes_literal_wire_key_and_structural_resolution() {
    let bytes = current_bytes();
    let record = PositionEpisodeCurrentRecordV1::decode(&bytes).unwrap();
    let key = PositionEpisodeCurrentRecordV1::state_key(&account(), &market()).unwrap();
    assert_eq!(key.namespace(), "position-episode-current.v1");
    assert_eq!(key.key(), CURRENT_KEY_GOLDEN);
    assert_eq!(
        PositionEpisodeCurrentRecordV1::decode_at(&key, &bytes).unwrap(),
        record
    );
    assert_eq!(record.account_id(), account());
    assert_eq!(record.market_id(), &market());
    assert_eq!(record.episode_id(), Some(&episode_id()));
    assert_eq!(
        record.attribution_resolution(),
        EpisodeAttributionResolutionV1::Resolved
    );
    assert_eq!(record.last_event_id(), &opening_event());
    assert_eq!(record.last_block_height(), BlockHeight::new(1600));
    let wrong_key = PositionEpisodeCurrentRecordV1::state_key(
        &Address::parse_api(OTHER_ACCOUNT).unwrap(),
        &market(),
    )
    .unwrap();
    assert_eq!(
        PositionEpisodeCurrentRecordV1::decode_at(&wrong_key, &bytes),
        Err(PositionStateError::KeyMismatch)
    );

    let no_open = String::from_utf8(bytes)
        .unwrap()
        .replace(
            &format!(r#""episode_id":"{EPISODE_ID}""#),
            r#""episode_id":null"#,
        )
        .replace(
            r#""attribution_resolution":"resolved""#,
            r#""attribution_resolution":"no_open_episode""#,
        );
    assert_eq!(
        PositionEpisodeCurrentRecordV1::decode(no_open.as_bytes())
            .unwrap()
            .attribution_resolution(),
        EpisodeAttributionResolutionV1::NoOpenEpisode
    );
}

#[test]
fn effect_record_freezes_literal_wire_key_ordinals_and_accessors() {
    let bytes = opened_effect_bytes();
    let record = PositionEpisodeEffectFactRecordV1::decode(&bytes).unwrap();
    let key =
        PositionEpisodeEffectFactRecordV1::state_key(&opening_event(), &account(), &market(), 0)
            .unwrap();
    assert_eq!(key.namespace(), "position-episode-effect-fact.v1");
    assert_eq!(key.key(), EFFECT_KEY_GOLDEN);
    assert_eq!(
        PositionEpisodeEffectFactRecordV1::decode_at(&key, &bytes).unwrap(),
        record
    );
    assert_eq!(record.event_id(), &opening_event());
    assert_eq!(record.account_id(), account());
    assert_eq!(record.market_id(), &market());
    assert_eq!(record.leg_ordinal(), 0);
    assert_eq!(record.episode_id(), &episode_id());
    assert_eq!(record.effect_kind(), EpisodeEffectKindV1::Opened);
    assert_eq!(record.buy_quantity_delta().to_string(), "0.25000000");
    assert_eq!(record.buy_notional_delta().to_string(), "16250");
    assert_eq!(record.sell_quantity_delta().to_string(), "0.00000000");
    assert_eq!(record.sell_notional_delta().to_string(), "0");
    assert_eq!(record.funding_paid_delta().to_string(), "0.000000");
    assert_eq!(record.funding_received_delta().to_string(), "0.000000");
    assert_eq!(record.close_cause(), None);
    assert_eq!(record.rule_version(), RULE_VERSION);

    let second_key =
        PositionEpisodeEffectFactRecordV1::state_key(&opening_event(), &account(), &market(), 1)
            .unwrap();
    assert_ne!(key, second_key);
    assert_eq!(
        PositionEpisodeEffectFactRecordV1::decode_at(&second_key, &bytes),
        Err(PositionStateError::KeyMismatch)
    );
    assert_eq!(
        PositionEpisodeEffectFactRecordV1::state_key(&opening_event(), &account(), &market(), 2,),
        Err(PositionStateError::InvalidRecord)
    );
}

#[test]
fn codecs_reject_identity_schema_canonicality_and_structural_mismatches() {
    let episode = String::from_utf8(open_episode_bytes()).unwrap();
    assert_eq!(
        PositionEpisodeRecordV1::decode(
            episode
                .replace(
                    EPISODE_ID,
                    "pos_ep_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )
                .as_bytes(),
        ),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        PositionEpisodeRecordV1::decode(
            episode
                .replace(
                    "hyperliquid-alpha-desk/position-episode/v1",
                    "hyperliquid-alpha-desk/position-episode/v2",
                )
                .as_bytes(),
        ),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        PositionEpisodeRecordV1::decode(format!(" {episode}").as_bytes()),
        Err(PositionStateError::NonCanonical)
    );
    let mut unknown = episode.clone();
    unknown.pop();
    unknown.push_str(r#","extra":true}"#);
    assert_eq!(
        PositionEpisodeRecordV1::decode(unknown.as_bytes()),
        Err(PositionStateError::Codec)
    );

    let wrong_key = PositionEpisodeRecordV1::state_key(
        &derive_position_episode_id(
            &account(),
            &MarketId::new("perp:ETH").unwrap(),
            &opening_event(),
            0,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        PositionEpisodeRecordV1::decode_at(&wrong_key, episode.as_bytes()),
        Err(PositionStateError::KeyMismatch)
    );

    let current = String::from_utf8(current_bytes()).unwrap();
    assert_eq!(
        PositionEpisodeCurrentRecordV1::decode(
            current
                .replace(
                    r#""attribution_resolution":"resolved""#,
                    r#""attribution_resolution":"interrupted""#,
                )
                .as_bytes(),
        ),
        Err(PositionStateError::InvalidRecord)
    );
}

#[test]
fn episode_and_effect_invariants_fail_closed() {
    let open = String::from_utf8(open_episode_bytes()).unwrap();
    let invalid_episodes = [
        open.replace(
            r#""opening_position":"0.00000000""#,
            r#""opening_position":"1.00000000""#,
        ),
        open.replace(
            r#""completeness":"complete_from_flat""#,
            r#""completeness":"partial_from_first_observation""#,
        ),
        open.replace(
            r#""buy_quantity":"0.25000000""#,
            r#""buy_quantity":"-0.25000000""#,
        ),
        open.replace(r#""buy_notional":"16250""#, r#""buy_notional":"-16250""#),
        open.replace(
            r#""buy_quantity":"0.25000000""#,
            r#""buy_quantity":"0.00000000""#,
        ),
        open.replace(r#""buy_notional":"16250""#, r#""buy_notional":"0""#),
        open.replace(
            r#""funding_paid":"0.000000""#,
            r#""funding_paid":"-0.000001""#,
        ),
        open.replace(
            r#""close_event_id":null"#,
            r#""close_event_id":"evt-close""#,
        ),
        open.replace(r#""status":"open""#, r#""status":"closed""#),
        open.replace(r#""status":"open""#, r#""status":"closed""#)
            .replace(
                r#""close_event_id":null"#,
                r#""close_event_id":"evt-close""#,
            )
            .replace(r#""close_cause":null"#, r#""close_cause":"settlement""#),
        open.replace(r#""status":"open""#, r#""status":"interrupted""#)
            .replace(
                r#""close_event_id":null"#,
                r#""close_event_id":"evt-close""#,
            )
            .replace(r#""close_cause":null"#, r#""close_cause":"trade_flat""#),
    ];
    for bytes in invalid_episodes {
        assert_eq!(
            PositionEpisodeRecordV1::decode(bytes.as_bytes()),
            Err(PositionStateError::InvalidRecord),
            "{bytes}"
        );
    }

    let effect = String::from_utf8(opened_effect_bytes()).unwrap();
    let invalid_effects = [
        effect.replace(r#""leg_ordinal":0"#, r#""leg_ordinal":2"#),
        effect.replace(
            r#""buy_quantity_delta":"0.25000000""#,
            r#""buy_quantity_delta":"-0.25000000""#,
        ),
        effect.replace(
            r#""buy_notional_delta":"16250""#,
            r#""buy_notional_delta":"0""#,
        ),
        effect.replace(r#""close_cause":null"#, r#""close_cause":"trade_flat""#),
        effect.replace(
            RULE_VERSION,
            "hyperliquid-alpha-desk-canonical-position-episode@2.0.0",
        ),
        effect
            .replace(r#""effect_kind":"opened""#, r#""effect_kind":"closed""#)
            .replace(r#""close_cause":null"#, r#""close_cause":"settlement""#),
        effect
            .replace(
                r#""effect_kind":"opened""#,
                r#""effect_kind":"interrupted""#,
            )
            .replace(r#""close_cause":null"#, r#""close_cause":"trade_flat""#),
    ];
    for bytes in invalid_effects {
        assert_eq!(
            PositionEpisodeEffectFactRecordV1::decode(bytes.as_bytes()),
            Err(PositionStateError::InvalidRecord),
            "{bytes}"
        );
    }

    let closed = open
        .replace(
            r#""close_event_id":null"#,
            r#""close_event_id":"evt-close""#,
        )
        .replace(r#""close_cause":null"#, r#""close_cause":"trade_flat""#)
        .replace(r#""status":"open""#, r#""status":"closed""#)
        .replace(
            r#""last_event_id":"evt-open-fixed""#,
            r#""last_event_id":"evt-close""#,
        );
    let closed_record = PositionEpisodeRecordV1::decode(closed.as_bytes()).unwrap();
    assert_eq!(closed_record.status(), EpisodeStatusV1::Closed);
    assert_eq!(
        closed_record.close_cause(),
        Some(EpisodeCloseCauseV1::TradeFlat)
    );

    let interrupted = closed
        .replace(
            r#""close_cause":"trade_flat""#,
            r#""close_cause":"settlement""#,
        )
        .replace(r#""status":"closed""#, r#""status":"interrupted""#);
    assert_eq!(
        PositionEpisodeRecordV1::decode(interrupted.as_bytes())
            .unwrap()
            .status(),
        EpisodeStatusV1::Interrupted
    );
}

#[test]
fn every_frozen_enum_wire_variant_is_accepted_only_in_its_valid_matrix() {
    let open = String::from_utf8(open_episode_bytes()).unwrap();
    let partial = open
        .replace(
            r#""opening_position":"0.00000000""#,
            r#""opening_position":"1.00000000""#,
        )
        .replace(
            r#""completeness":"complete_from_flat""#,
            r#""completeness":"partial_from_first_observation""#,
        );
    assert_eq!(
        PositionEpisodeRecordV1::decode(partial.as_bytes())
            .unwrap()
            .completeness(),
        EpisodeCompletenessV1::PartialFromFirstObservation
    );

    for cause in ["trade_flat", "trade_reversal"] {
        let closed = open
            .replace(
                r#""close_event_id":null"#,
                r#""close_event_id":"evt-close""#,
            )
            .replace(
                r#""close_cause":null"#,
                &format!(r#""close_cause":"{cause}""#),
            )
            .replace(r#""status":"open""#, r#""status":"closed""#)
            .replace(
                r#""last_event_id":"evt-open-fixed""#,
                r#""last_event_id":"evt-close""#,
            );
        assert_eq!(
            PositionEpisodeRecordV1::decode(closed.as_bytes())
                .unwrap()
                .status(),
            EpisodeStatusV1::Closed
        );
    }
    for cause in ["liquidation_fill", "settlement", "backstop_interrupted"] {
        let interrupted = open
            .replace(
                r#""close_event_id":null"#,
                r#""close_event_id":"evt-interrupt""#,
            )
            .replace(
                r#""close_cause":null"#,
                &format!(r#""close_cause":"{cause}""#),
            )
            .replace(r#""status":"open""#, r#""status":"interrupted""#)
            .replace(
                r#""last_event_id":"evt-open-fixed""#,
                r#""last_event_id":"evt-interrupt""#,
            );
        assert_eq!(
            PositionEpisodeRecordV1::decode(interrupted.as_bytes())
                .unwrap()
                .status(),
            EpisodeStatusV1::Interrupted
        );
    }

    let current = String::from_utf8(current_bytes()).unwrap();
    for resolution in ["no_open_episode", "interrupted"] {
        let bytes = current
            .replace(
                &format!(r#""episode_id":"{EPISODE_ID}""#),
                r#""episode_id":null"#,
            )
            .replace(
                r#""attribution_resolution":"resolved""#,
                &format!(r#""attribution_resolution":"{resolution}""#),
            );
        let record = PositionEpisodeCurrentRecordV1::decode(bytes.as_bytes()).unwrap();
        assert_eq!(record.episode_id(), None);
    }

    let opened = String::from_utf8(opened_effect_bytes()).unwrap();
    for kind in ["opened", "updated"] {
        let bytes = opened.replace(
            r#""effect_kind":"opened""#,
            &format!(r#""effect_kind":"{kind}""#),
        );
        assert!(PositionEpisodeEffectFactRecordV1::decode(bytes.as_bytes()).is_ok());
    }
    for (kind, causes) in [
        ("closed", &["trade_flat", "trade_reversal"][..]),
        (
            "interrupted",
            &["liquidation_fill", "settlement", "backstop_interrupted"][..],
        ),
    ] {
        for cause in causes {
            let bytes = opened
                .replace(
                    r#""effect_kind":"opened""#,
                    &format!(r#""effect_kind":"{kind}""#),
                )
                .replace(
                    r#""close_cause":null"#,
                    &format!(r#""close_cause":"{cause}""#),
                );
            let record = PositionEpisodeEffectFactRecordV1::decode(bytes.as_bytes()).unwrap();
            assert_eq!(
                record.effect_kind(),
                if kind == "closed" {
                    EpisodeEffectKindV1::Closed
                } else {
                    EpisodeEffectKindV1::Interrupted
                }
            );
        }
    }
}

#[test]
fn exact_notional_boundaries_are_revalidated_by_episode_codecs() {
    let open = String::from_utf8(open_episode_bytes()).unwrap();
    let digit_155 = format!("1{}", "0".repeat(154));
    let scale_76 = format!("0.{}1", "0".repeat(75));
    for boundary in [&digit_155, &scale_76] {
        let bytes = open.replace(
            r#""buy_notional":"16250""#,
            &format!(r#""buy_notional":"{boundary}""#),
        );
        assert_eq!(
            PositionEpisodeRecordV1::decode(bytes.as_bytes())
                .unwrap()
                .buy_notional()
                .to_string(),
            *boundary
        );
    }

    let maximum_512_bit =
        "1340780792994259709957402499820584612747936582059239337772356144372176403007\
         3546976801874298166903427690031858186486050853753882811946569946433649006084095"
            .replace(' ', "");
    let bytes = open.replace(
        r#""buy_notional":"16250""#,
        &format!(r#""buy_notional":"{maximum_512_bit}""#),
    );
    assert!(PositionEpisodeRecordV1::decode(bytes.as_bytes()).is_ok());

    let invalid = [
        "1".repeat(156),
        "1".repeat(257),
        format!("0.{}1", "0".repeat(76)),
        "1340780792994259709957402499820584612747936582059239337772356144372176403007\
         3546976801874298166903427690031858186486050853753882811946569946433649006084096"
            .replace(' ', ""),
    ];
    for value in invalid {
        let bytes = open.replace(
            r#""buy_notional":"16250""#,
            &format!(r#""buy_notional":"{value}""#),
        );
        assert_eq!(
            PositionEpisodeRecordV1::decode(bytes.as_bytes()),
            Err(PositionStateError::InvalidRecord),
            "{}",
            value.len()
        );
    }
}

#[test]
fn record_and_variable_key_bounds_are_inclusive_and_preallocated() {
    fn episode_bytes_for_market(market_id: &str) -> Vec<u8> {
        let market = MarketId::new(market_id).unwrap();
        let episode_id =
            derive_position_episode_id(&account(), &market, &opening_event(), 0).unwrap();
        format!(
            concat!(
                r#"{{"schema":"hyperliquid-alpha-desk/position-episode/v1","#,
                r#""episode_id":"{}","#,
                r#""account_id":"{ACCOUNT}","#,
                r#""market_id":"{}","#,
                r#""opening_anchor_event_id":"{OPEN_EVENT}","#,
                r#""opening_leg_ordinal":0,"#,
                r#""opening_position":"0.00000000","#,
                r#""close_event_id":null,"#,
                r#""close_cause":null,"#,
                r#""completeness":"complete_from_flat","#,
                r#""buy_quantity":"0.25000000","#,
                r#""buy_notional":"16250","#,
                r#""sell_quantity":"0.00000000","#,
                r#""sell_notional":"0","#,
                r#""funding_paid":"0.000000","#,
                r#""funding_received":"0.000000","#,
                r#""status":"open","#,
                r#""last_event_id":"{OPEN_EVENT}","#,
                r#""last_block_height":1600}}"#
            ),
            episode_id.as_str(),
            market_id,
            ACCOUNT = ACCOUNT,
            OPEN_EVENT = OPEN_EVENT,
        )
        .into_bytes()
    }

    let base = episode_bytes_for_market("m");
    let exact_market = format!("m{}", "x".repeat(16 * 1024 - base.len()));
    let exact = episode_bytes_for_market(&exact_market);
    assert_eq!(exact.len(), 16 * 1024);
    assert!(PositionEpisodeRecordV1::decode(&exact).is_ok());

    let over = episode_bytes_for_market(&format!("{exact_market}x"));
    assert_eq!(over.len(), 16 * 1024 + 1);
    assert_eq!(
        PositionEpisodeRecordV1::decode(&over),
        Err(PositionStateError::LimitExceeded)
    );

    let current_market = MarketId::new("m".repeat(65_500)).unwrap();
    let current_key =
        PositionEpisodeCurrentRecordV1::state_key(&account(), &current_market).unwrap();
    assert_eq!(current_key.key().len(), 64 * 1024);
    assert_eq!(
        PositionEpisodeCurrentRecordV1::state_key(
            &account(),
            &MarketId::new("m".repeat(65_501)).unwrap(),
        ),
        Err(PositionStateError::InvalidKey)
    );

    let effect_market = MarketId::new("m".repeat(65_482)).unwrap();
    let effect_key = PositionEpisodeEffectFactRecordV1::state_key(
        &EventId::new("e").unwrap(),
        &account(),
        &effect_market,
        0,
    )
    .unwrap();
    assert_eq!(effect_key.key().len(), 64 * 1024);
    assert_eq!(
        PositionEpisodeEffectFactRecordV1::state_key(
            &EventId::new("e").unwrap(),
            &account(),
            &MarketId::new("m".repeat(65_483)).unwrap(),
            0,
        ),
        Err(PositionStateError::InvalidKey)
    );

    let derivation_market = MarketId::new("m".repeat(65_490)).unwrap();
    assert!(
        derive_position_episode_id(
            &account(),
            &derivation_market,
            &EventId::new("e").unwrap(),
            0,
        )
        .is_ok()
    );
    assert_eq!(
        derive_position_episode_id(
            &account(),
            &MarketId::new("m".repeat(65_491)).unwrap(),
            &EventId::new("e").unwrap(),
            0,
        ),
        Err(PositionStateError::InvalidKey)
    );
}
