//! Dolby Vision in the master playlist: `dv=1`, `Dovi::signal`.

use std::sync::Arc;

use super::*;
use crate::media::Media;

fn view(media: Media, query: &str, transcode: bool) -> View {
    let opts = StreamOptions::from(
        &query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    );
    let v = media.video.clone().unwrap();
    let variants = crate::media::transcode::ladder_with(&crate::media::hwaccel::Hw::nvenc(), &v, opts.hdr_display, None, transcode);
    View {
        sid: "s".into(),
        owner: None,
        moonrise_item: None,
        media: Arc::new(media),
        opts,
        variants: Arc::new(variants),
        selected: 0,
    }
}

fn dv_media(profile: u8, compat: u8) -> Media {
    let mut v = crate::media::transcode::tests::video("hevc", 3840, 2160, 30_000_000, "PQ");
    v.codecs_attr = Some("hvc1.2.4.L150.B0".into());
    v.dovi = Some(crate::media::Dovi {
        profile,
        level: 6,
        rpu: true,
        el: false,
        bl: true,
        compat,
    });
    v.dovi_only = profile == 5;
    crate::media::transcode::tests::media(v)
}

fn stream_inf(master: &str) -> Vec<&str> {
    master.lines().filter(|l| l.starts_with("#EXT-X-STREAM-INF")).collect()
}

#[test]
fn dolby_vision_only_for_players_that_show_it() {
    // Profile 8.1, a player that shows Dolby Vision: the base layer's codec, Dolby Vision as
    // the supplemental one.
    let out = master(&view(dv_media(8, 1), "hdr=1&dv=1", false));
    let inf = stream_inf(&out);
    assert_eq!(inf.len(), 1);
    assert!(
        inf[0].contains("CODECS=\"hvc1.2.4.L150.B0\",SUPPLEMENTAL-CODECS=\"dvh1.08.06/db1p\",") && inf[0].contains("VIDEO-RANGE=PQ"),
        "{out}"
    );
    // 8.4: HLG compatible.
    let out = master(&view(dv_media(8, 4), "hdr=1&dv=1", false));
    assert!(out.contains("SUPPLEMENTAL-CODECS=\"dvh1.08.06/db4h\""), "{out}");
    // Profile 5: Dolby Vision is the codec.
    let out = master(&view(dv_media(5, 0), "hdr=1&dv=1", false));
    assert!(
        out.contains("CODECS=\"dvh1.05.06\",RESOLUTION") && !out.contains("SUPPLEMENTAL"),
        "{out}"
    );

    // Without `dv=1`, or on a display without HDR: as before.
    for q in ["hdr=1", "hdr=0&dv=1"] {
        for p in [(8, 1), (5, 0)] {
            let out = master(&view(dv_media(p.0, p.1), q, false));
            assert!(
                !out.contains("SUPPLEMENTAL") && !out.contains("dvh1") && out.contains("CODECS=\"hvc1.2.4.L150.B0\""),
                "{q} {p:?}: {out}"
            );
        }
    }
    // Profile 7 (a second layer): never signalled.
    let mut m = dv_media(7, 6);
    m.video.as_mut().unwrap().dovi.as_mut().unwrap().el = true;
    assert!(!master(&view(m, "hdr=1&dv=1", false)).contains("dvh1"));
}

#[test]
fn dolby_vision_never_on_transcodes() {
    // The ladder's transcodes (listed with abr) carry no Dolby Vision metadata.
    let v = view(dv_media(8, 1), "hdr=1&dv=1&abr=1", true);
    assert!(v.variants.len() > 1 && v.variants[0].is_copy());
    let out = master(&v);
    let inf = stream_inf(&out);
    assert!(inf.len() > 1, "{out}");
    assert!(inf[0].contains("SUPPLEMENTAL-CODECS"), "{out}");
    assert!(inf[1..].iter().all(|l| !l.contains("SUPPLEMENTAL") && !l.contains("dvh1")), "{out}");
}
