use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoError, InfoObservationKind, InfoParseContext, UserAbstraction, UserRole,
    parse_portfolio, parse_sub_accounts, parse_user_abstraction, parse_user_dex_abstraction,
    parse_user_non_funding_ledger_updates, parse_user_rate_limit, parse_user_role,
    parse_user_to_multi_sig_signers,
};
use serde_json::json;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t06-accounts"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t06-accounts").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_role_variants_preserve_relationships_and_do_not_conflate_addresses() {
    let agent = parse_user_role(&read_fixture("response-user-role-agent.json"), context())
        .expect("agent")
        .1;
    match &agent {
        UserRole::Agent { user } => {
            assert_eq!(
                user.to_api_string(),
                "0x1111111111111111111111111111111111111111"
            );
        }
        other => panic!("expected agent, got {other:?}"),
    }

    let sub = parse_user_role(
        &read_fixture("response-user-role-subaccount.json"),
        context(),
    )
    .expect("sub")
    .1;
    match &sub {
        UserRole::SubAccount { master } => {
            assert_eq!(
                master.to_api_string(),
                "0x2222222222222222222222222222222222222222"
            );
            assert_ne!(
                master.to_api_string(),
                agent.related_account().expect("agent user").to_api_string()
            );
        }
        other => panic!("expected subaccount, got {other:?}"),
    }

    let user = parse_user_role(br#"{"role":"user"}"#, context())
        .expect("user")
        .1;
    assert!(matches!(user, UserRole::User));
    assert!(user.related_account().is_none());

    let missing = parse_user_role(br#"{"role":"missing"}"#, context())
        .expect("missing")
        .1;
    assert!(matches!(missing, UserRole::Missing));

    let vault = parse_user_role(br#"{"role":"vault"}"#, context())
        .expect("vault")
        .1;
    assert!(matches!(vault, UserRole::Vault));

    let error = parse_user_role(br#"{"role":"copyTrader"}"#, context()).expect_err("unknown");
    assert!(matches!(
        error,
        InfoError::UnknownStateAffectingVariant { .. }
    ));
}

#[test]
fn info_subaccounts_keep_master_and_sub_distinct() {
    let accounts = parse_sub_accounts(&read_fixture("response-sub-accounts.json"), context())
        .expect("subs")
        .1;
    let row = &accounts.accounts()[0];
    assert_ne!(row.master(), row.sub_account_user());
    assert_eq!(
        row.master().to_api_string(),
        "0x8c967e73e6b15087c42a10d344cff4c96d877f1d"
    );

    let conflated = json!([{
        "name": "bad",
        "subAccountUser": "0x1111111111111111111111111111111111111111",
        "master": "0x1111111111111111111111111111111111111111",
        "clearinghouseState": {},
        "spotState": {}
    }]);
    let error = parse_sub_accounts(&serde_json::to_vec(&conflated).expect("encode"), context())
        .expect_err("same address");
    assert!(matches!(error, InfoError::MalformedPayload { .. }));
}

#[test]
fn info_portfolio_rate_limit_abstraction_and_multisig_parse() {
    let portfolio = parse_portfolio(&read_fixture("response-portfolio.json"), context())
        .expect("portfolio")
        .1;
    assert_eq!(portfolio.kind(), InfoObservationKind::ReconciledSnapshot);
    assert_eq!(portfolio.windows()[0].period(), "day");

    let limit = parse_user_rate_limit(&read_fixture("response-user-rate-limit.json"), context())
        .expect("rate")
        .1;
    assert_eq!(limit.n_requests_used(), 2890);

    let abstraction =
        parse_user_abstraction(&read_fixture("response-user-abstraction.json"), context())
            .expect("abstraction")
            .1;
    assert_eq!(abstraction, UserAbstraction::UnifiedAccount);

    let dex = parse_user_dex_abstraction(
        &read_fixture("response-user-dex-abstraction.json"),
        context(),
    )
    .expect("dex")
    .1;
    assert!(dex.enabled());

    let signers = parse_user_to_multi_sig_signers(
        &read_fixture("response-user-to-multi-sig-signers.json"),
        context(),
    )
    .expect("msig")
    .1
    .expect("present");
    assert_eq!(signers.threshold(), 2);
    assert_eq!(signers.authorized_users().len(), 2);

    let none = parse_user_to_multi_sig_signers(b"null", context())
        .expect("null")
        .1;
    assert!(none.is_none());
}

#[test]
fn info_non_funding_ledger_updates_quarantine_unknown_delta() {
    let updates = parse_user_non_funding_ledger_updates(
        &read_fixture("response-user-non-funding-ledger.json"),
        context(),
    )
    .expect("ledger")
    .1;
    assert_eq!(updates.kind(), InfoObservationKind::BoundedHistory);
    assert_eq!(updates.updates()[0].delta_type(), "deposit");
    assert_eq!(updates.updates()[1].delta_type(), "spotTransfer");

    let raw = serde_json::to_vec(&json!([{
        "time": 1,
        "hash": "0x00",
        "delta": {"type": "mysteryDelta"}
    }]))
    .expect("encode");
    let error = parse_user_non_funding_ledger_updates(&raw, context()).expect_err("unknown");
    assert!(matches!(
        error,
        InfoError::UnknownStateAffectingVariant { .. }
    ));
}
