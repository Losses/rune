//! Feeds the parsers many generated, mostly malformed inputs. None of them may
//! panic, and LRC lines must come out sorted with an end no earlier than the
//! start. The generator is seeded, so a failure reproduces. Set
//! `LYRIC_ROBUSTNESS_ROUNDS` to run longer.

use std::panic::{AssertUnwindSafe, catch_unwind};

use lyric::{lrc::parse_lrc, srt::parse_srt, ttml::parse_ttml, types::LyricFile, vtt::parse_vtt};

fn rounds() -> usize {
    std::env::var("LYRIC_ROBUSTNESS_ROUNDS")
        .ok()
        .and_then(|x| x.parse().ok())
        .unwrap_or(5_000)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const PIECES: &[&str] = &[
    "[",
    "]",
    "<",
    ">",
    ":",
    ".",
    ",",
    "-",
    "+",
    " ",
    "\n",
    "\r\n",
    "\t",
    "\u{feff}",
    "0",
    "1",
    "5",
    "9",
    "59",
    "99",
    "999",
    "00:01",
    "00:01.5",
    "00:01.50",
    "00:01.500",
    "01:00:01.00",
    "00:01:50",
    "ti",
    "offset",
    "M:",
    "F:",
    "D:",
    "歌",
    "の",
    "a",
    "I <3 you",
    "-->",
    "WEBVTT",
    "<b>",
    "</i>",
    "&amp;",
    "\"",
    "=",
];

fn noise(rng: &mut Rng) -> String {
    (0..rng.below(60)).map(|_| rng.pick(PIECES)).collect()
}

fn time(rng: &mut Rng) -> String {
    let fraction = ["", ".5", ".50", ".500", ".5000", ":50", "."][rng.below(7)];
    let mut t = format!("{:02}:{:02}{fraction}", rng.below(120), rng.below(70));
    if rng.below(8) == 0 {
        t = format!("{:02}:{t}", rng.below(3));
    }
    if rng.below(16) == 0 {
        t = noise(rng);
    }
    t
}

fn lrc(rng: &mut Rng) -> String {
    let mut out = String::new();
    if rng.below(4) == 0 {
        out.push('\u{feff}');
    }
    for _ in 0..rng.below(12) {
        match rng.below(6) {
            0 => out += &format!("[{}:{}]", rng.pick(&["ti", "ar", "offset", ""]), noise(rng)),
            1 => out += &noise(rng),
            _ => {
                for _ in 0..1 + rng.below(3) {
                    out += &format!("[{}]", time(rng));
                }
                for _ in 0..rng.below(5) {
                    if rng.below(3) > 0 {
                        out += &format!("<{}>", time(rng));
                    }
                    out += rng.pick(&["word ", "の", "", " ", "<3", "M:", "a b"]);
                }
            }
        }
        out += rng.pick(&["\n", "\r\n", ""]);
    }
    out
}

fn ttml(rng: &mut Rng) -> String {
    let mut out = String::from("<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><div>");
    for _ in 0..rng.below(6) {
        let timing = |rng: &mut Rng| {
            let value = match rng.below(4) {
                0 => format!("{}s", rng.below(300)),
                1 => format!("{}ms", rng.below(300_000)),
                _ => time(rng),
            };
            format!(" {}=\"{value}\"", rng.pick(&["begin", "end", "dur"]))
        };
        out += &format!("<p{}{}>", timing(rng), timing(rng));
        for _ in 0..rng.below(5) {
            match rng.below(4) {
                0 => out += &noise(rng).replace(['<', '>', '&'], ""),
                1 => out += "<br/>",
                _ => {
                    let role =
                        rng.pick(&["", " ttm:role=\"x-translation\"", " ttm:role=\"x-roman\""]);
                    out += &format!(
                        "<span{}{role}>{}</span> ",
                        timing(rng),
                        rng.pick(&["a", "の", " "])
                    );
                }
            }
        }
        out += "</p>";
    }
    out + "</div></body></tt>"
}

fn cues(rng: &mut Rng, header: &str, separator: &str) -> String {
    let mut out = String::from(header);
    for i in 0..rng.below(6) {
        out += &format!(
            "{i}\n{} --> {}\n{}\n\n",
            time(rng).replace('.', separator),
            time(rng).replace('.', separator),
            noise(rng)
        );
    }
    if rng.below(2) == 0 {
        out = out.trim_end().to_string();
    }
    out
}

fn check(label: &str, input: &str, parse: fn(&str) -> anyhow::Result<LyricFile>, sorted: bool) {
    let result = catch_unwind(AssertUnwindSafe(|| parse(input)));
    let Ok(result) = result else {
        panic!("{label} panicked on {input:?}");
    };
    let Ok(file) = result else { return };

    let mut previous = i32::MIN;
    for line in &file.lyrics {
        let (start, end) = (
            i32::from(line.start_time.clone()),
            i32::from(line.end_time.clone()),
        );
        assert!(
            end >= start,
            "{label}: line ends before it starts in {input:?}"
        );
        if sorted {
            assert!(
                start >= previous,
                "{label}: lines out of order in {input:?}"
            );
        }
        previous = start;
        for (s, e, _) in &line.word_time_tags {
            let _ = (i32::from(s.clone()), i32::from(e.clone()));
        }
    }
}

#[test]
fn generated_inputs_do_not_break_the_parsers() {
    let mut rng = Rng(0x5eed_1a7e_0000_0001);
    for _ in 0..rounds() {
        let input = lrc(&mut rng);
        check("lrc", &input, parse_lrc, true);
        let input = noise(&mut rng);
        check("lrc noise", &input, parse_lrc, true);
        check("ttml noise", &input, parse_ttml, false);
        check("vtt noise", &input, parse_vtt, false);
        check("srt noise", &input, parse_srt, false);
        check("ttml", &ttml(&mut rng), parse_ttml, false);
        check("vtt", &cues(&mut rng, "WEBVTT\n\n", "."), parse_vtt, false);
        check("srt", &cues(&mut rng, "", ","), parse_srt, false);
    }
}
