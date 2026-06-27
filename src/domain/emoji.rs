//! Sendable emojis for the reaction picker.
//!
//! Two sources are merged: the team's **custom** emojis (from
//! `keybase chat api emojilist`) and a bundled **curated** set of common
//! standard emojis — Keybase's `emojilist` returns only the custom ones,
//! so the standard glyphs (👍, ❤️, …) have to come from here.

/// One sendable emoji.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Emoji {
    /// Shortcode without colons (e.g. `+1`, `fire`, a team's `partyparrot`).
    /// Sent as the reaction body, wrapped in colons.
    pub alias: String,
    /// What the picker shows: the unicode glyph for stock emojis, or
    /// `:alias:` for custom (image-hosted) ones the terminal can't draw.
    pub display: String,
    /// Lowercased space-separated search terms (alias + synonyms) the
    /// picker filters on, so "thumb" finds 👍 even though its alias is `+1`.
    pub keywords: String,
}

impl Emoji {
    /// Builds a curated standard emoji.
    fn std(display: &str, alias: &str, keywords: &str) -> Self {
        Self {
            alias: alias.to_string(),
            display: display.to_string(),
            keywords: format!("{alias} {keywords}"),
        }
    }
}

/// A curated set of common standard emojis to react with. Keybase doesn't
/// serve the stock unicode set, so this covers the everyday reactions; the
/// team's custom emojis are merged on top at runtime.
pub fn standard() -> Vec<Emoji> {
    [
        ("👍", "+1", "thumbsup thumb up like yes approve ok good"),
        ("👎", "-1", "thumbsdown thumb down dislike no bad"),
        ("❤️", "heart", "love red"),
        ("🧡", "orange_heart", "love orange"),
        ("💛", "yellow_heart", "love yellow"),
        ("💚", "green_heart", "love green"),
        ("💙", "blue_heart", "love blue"),
        ("💜", "purple_heart", "love purple"),
        ("🖤", "black_heart", "love black"),
        ("💔", "broken_heart", "heartbroken sad"),
        ("🔥", "fire", "lit hot flame"),
        ("🎉", "tada", "party celebrate congrats hooray"),
        ("🎊", "confetti_ball", "party celebrate confetti"),
        ("😂", "joy", "laugh lol haha funny tears"),
        ("🤣", "rofl", "rolling laughing lmao"),
        ("😅", "sweat_smile", "nervous phew relief"),
        ("😄", "smile", "happy grin"),
        ("😁", "grin", "happy teeth"),
        ("😊", "blush", "happy smile shy"),
        ("🙂", "slightly_smiling_face", "smile slight"),
        ("😉", "wink", "winking"),
        ("😍", "heart_eyes", "love adore in love"),
        ("🥰", "smiling_face_with_three_hearts", "love adore"),
        ("😘", "kissing_heart", "kiss love"),
        ("😎", "sunglasses", "cool"),
        ("🤩", "star_struck", "amazed wow stars excited"),
        ("🥳", "partying_face", "party celebrate"),
        ("🤔", "thinking", "think hmm consider"),
        ("🤨", "raised_eyebrow", "skeptic doubt suspicious"),
        ("😐", "neutral_face", "meh neutral"),
        ("😴", "sleeping", "sleep zzz tired"),
        ("😢", "cry", "sad tear"),
        ("😭", "sob", "crying bawl sad"),
        ("😡", "rage", "angry mad furious"),
        ("😠", "angry", "mad annoyed"),
        ("🤯", "exploding_head", "mind blown shocked"),
        ("😱", "scream", "shocked scared fear"),
        ("😬", "grimacing", "awkward yikes"),
        ("🙄", "roll_eyes", "eyeroll annoyed"),
        ("😏", "smirk", "smug"),
        ("😇", "innocent", "angel halo"),
        ("🤗", "hugs", "hug embrace"),
        ("🤫", "shushing_face", "quiet shh secret"),
        ("🤐", "zipper_mouth", "quiet silence"),
        ("😷", "mask", "sick ill"),
        ("🤮", "vomiting_face", "sick puke gross"),
        ("💀", "skull", "dead dying lol"),
        ("👻", "ghost", "boo spooky"),
        ("🤡", "clown_face", "clown joke"),
        ("💩", "poop", "crap shit poo"),
        ("🙏", "pray", "please thanks thank you hope blessed"),
        ("👀", "eyes", "look watching see"),
        ("👏", "clap", "applause bravo well done"),
        ("🙌", "raised_hands", "praise hooray celebrate"),
        ("🤝", "handshake", "deal agree shake"),
        ("💪", "muscle", "strong flex biceps"),
        ("✊", "fist", "fist bump power"),
        ("👊", "facepunch", "punch fist bump"),
        ("🤙", "call_me_hand", "shaka hang loose"),
        ("✌️", "v", "peace victory"),
        ("🤞", "crossed_fingers", "luck hope"),
        ("👌", "ok_hand", "okay perfect"),
        ("👋", "wave", "hello hi bye goodbye"),
        ("🤦", "facepalm", "facepalm ugh"),
        ("🤷", "shrug", "dunno whatever idk"),
        (
            "✅",
            "white_check_mark",
            "check tick done yes correct complete",
        ),
        ("☑️", "ballot_box_with_check", "check tick done"),
        ("✔️", "heavy_check_mark", "check tick yes"),
        ("❌", "x", "cross wrong no fail incorrect"),
        ("❎", "negative_squared_cross_mark", "cross no"),
        ("⭕", "o", "circle correct"),
        ("‼️", "bangbang", "exclamation important"),
        ("❓", "question", "question mark huh"),
        ("❗", "exclamation", "exclamation important warning"),
        ("💯", "100", "hundred perfect score keep it"),
        ("🚀", "rocket", "launch ship deploy fast"),
        ("⭐", "star", "favorite"),
        ("🌟", "star2", "glowing sparkle"),
        ("✨", "sparkles", "shiny clean new"),
        ("💫", "dizzy", "stars sparkle"),
        ("⚡", "zap", "lightning fast electric"),
        ("💥", "boom", "explosion collision"),
        ("☀️", "sunny", "sun sunny"),
        ("🌈", "rainbow", "rainbow pride"),
        ("🌚", "new_moon_with_face", "moon dark"),
        ("🎯", "dart", "target bullseye goal"),
        ("🏆", "trophy", "win winner champion award"),
        ("🥇", "first_place_medal", "gold win first"),
        ("🎁", "gift", "present gift"),
        ("💡", "bulb", "idea light"),
        ("📌", "pushpin", "pin location"),
        ("📎", "paperclip", "attach clip"),
        ("✏️", "pencil2", "write edit"),
        ("📝", "memo", "note write document"),
        ("🔔", "bell", "notification alert"),
        ("🔒", "lock", "locked secure private"),
        ("🔑", "key", "password secret"),
        ("💰", "moneybag", "money cash rich"),
        ("💸", "money_with_wings", "money spend cash"),
        ("🍺", "beer", "drink cheers"),
        ("🍻", "beers", "cheers drink toast"),
        ("☕", "coffee", "coffee drink"),
        ("🍕", "pizza", "food pizza"),
        ("🎂", "birthday", "cake birthday"),
        ("🤖", "robot", "bot robot ai"),
        ("👽", "alien", "alien ufo"),
        ("🐛", "bug", "bug insect error"),
        ("🐍", "snake", "snake python"),
        ("🦀", "crab", "crab rust"),
        ("🐙", "octopus", "octopus"),
        ("🦄", "unicorn", "unicorn magic rare"),
        ("🐶", "dog", "dog puppy"),
        ("🐱", "cat", "cat kitten"),
        ("❤️‍🔥", "heart_on_fire", "love fire passion"),
        ("💖", "sparkling_heart", "love sparkle"),
        ("💝", "gift_heart", "love gift"),
        ("🫶", "heart_hands", "love hands"),
        ("🫡", "saluting_face", "salute respect yes sir"),
        ("🙇", "bow", "bow sorry thanks respect"),
        ("👑", "crown", "king queen royal best"),
        ("⚠️", "warning", "warning caution alert"),
        ("🚨", "rotating_light", "alert siren emergency"),
        ("🆗", "ok", "okay ok"),
        ("🆒", "cool", "cool"),
        ("🆕", "new", "new"),
        ("🔝", "top", "top up best"),
        ("🤷‍♂️", "man_shrugging", "shrug dunno idk"),
        ("😆", "laughing", "haha laugh lol satisfied"),
        ("😋", "yum", "tasty delicious yummy"),
        ("🤤", "drooling_face", "drool want yum"),
        ("😜", "stuck_out_tongue_winking_eye", "tongue silly joke"),
        ("🤪", "zany_face", "crazy silly goofy"),
    ]
    .into_iter()
    .map(|(d, a, k)| Emoji::std(d, a, k))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_is_searchable_by_keyword_and_has_unique_aliases() {
        let s = standard();
        assert!(s.len() > 100, "expected a decent curated set");
        // "thumb" finds 👍 even though its alias is "+1".
        let like = s
            .iter()
            .find(|e| e.keywords.contains("thumb"))
            .expect("a thumbs-up emoji");
        assert_eq!(like.alias, "+1");
        assert_eq!(like.display, "👍");
        // The alias is always part of the search terms.
        assert!(s.iter().all(|e| e.keywords.contains(&e.alias)));
        // Aliases must be unique (so the runtime merge/dedupe is well-defined).
        let mut seen = std::collections::HashSet::new();
        for e in &s {
            assert!(
                seen.insert(e.alias.as_str()),
                "duplicate alias: {}",
                e.alias
            );
        }
    }
}
