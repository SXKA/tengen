//! Searching with extreme parameters must neither panic nor hang: every
//! constant of the search is a parameter, and tuning tries odd values.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run `pbrain-tengen --params <params> bench 5` and return its output.
fn bench(params: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pbrain-tengen"))
        .args(["--params", params, "bench", "5"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("pbrain-tengen starts");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("pbrain-tengen runs") {
            let output = child.wait_with_output().expect("output");
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            assert!(
                status.success(),
                "--params {params} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return text;
        }
        if start.elapsed() > Duration::from_secs(120) {
            child.kill().ok();
            panic!("--params {params} did not finish");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn extreme_parameters_search() {
    for params in [
        "lmr_depth=1",
        "root_lmr_depth=0,root_lmr_moves=0",
        "iir_depth=1,iir_reduction=5",
        "lmr_base=-500,lmr_scale=-100,lmr_cut_node=5",
        "se_depth=1,se_tt_depth=0,se_reduction=100",
        "alpha_reduce_min=0,alpha_reduce_max=100",
        "qsearch_plies=0,qsearch_plain_fours=0",
        "aspiration=0,aspiration_depth=0",
        "history_max=0,history_bonus=0",
        "time_falling_min=2000,time_falling_max=0",
        "fail_high_margin=0,fail_high_reduction=3",
        "nmp_depth=1,nmp_margin_base=-1000,nmp_margin_min=-1000,nmp_reduction_base=0,nmp_reduction_scale=0",
        "nmp_depth=0,nmp_margin_min=-5000,nmp_reduction_base=5000",
        "trivial_depth=100",
        "quick_win=1",
        "tt_eval=1,razor_depth=100,razor_base=-1000,razor_verify=0,rfp_margin=0",
        "lmr_frac=1,lmr_frac_bias=-1000,ext_single=300,ext_tt_pv=300,ext_tt=300,ext_near_four=300",
        "lmr_frac=1,lmr_frac_bias=1000,ext_single=-300,ext_tt=-300",
        "rfp_depth=100,rfp_margin=0,rfp_offset=-100,rfp_no_tt=1000,razor_depth=100,razor_verify=0",
        "probcut_depth=1,probcut_margin=-1000,probcut_reduction=0,probcut_moves=100",
        "probcut_depth=0,probcut_margin=100000,probcut_reduction=100",
        "threat_four_depth=100,threat_four_keep=100",
        "history_bonus=1000,history_bonus_lin=-1000,history_bonus_const=-100000,history_max=1,history_attack_malus=1,tt_move_history=1",
        "history_bonus=0,history_bonus_lin=100000,history_bonus_const=100000,history_bonus_max=100000,tt_move_history=1",
    ] {
        assert!(bench(params).contains("nodes"), "--params {params}");
    }
}
