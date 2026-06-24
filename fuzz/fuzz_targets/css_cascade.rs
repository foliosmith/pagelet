#![no_main]

use libfuzzer_sys::fuzz_target;
use pagelet::{
    core::ResourceLimits,
    document::ComputedStyle,
    epub::{cascade_css_for_element, parse_css, CssElementSnapshot},
};

const MAX_INPUT_LEN: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_LEN {
        return;
    }

    let input = String::from_utf8_lossy(data);
    let mut limits = ResourceLimits::mobile_defaults();
    limits.max_css_selectors = 1024;
    let Ok(stylesheet) = parse_css(&input, limits) else {
        return;
    };
    let element = CssElementSnapshot {
        name: "p".to_owned(),
        id: Some("target".to_owned()),
        classes: vec!["lead".to_owned(), "body".to_owned()],
        inline_style: Some("font-weight: bold".to_owned()),
    };
    let ancestors = vec![
        CssElementSnapshot {
            name: "section".to_owned(),
            id: Some("main".to_owned()),
            classes: vec!["chapter".to_owned()],
            inline_style: None,
        },
        CssElementSnapshot {
            name: "div".to_owned(),
            id: None,
            classes: vec!["content".to_owned()],
            inline_style: None,
        },
    ];
    let inherited = ComputedStyle::new().with_property("font-family", "serif");
    let _ = cascade_css_for_element(&element, &ancestors, &stylesheet, &inherited);
});
