//! The emoji catalogue: one table, three consumers.
//!
//! [`markdown::expand_emoji`](super::markdown::expand_emoji) resolves
//! `:shortcode:` tokens through [`find`], Tab-completion offers names through
//! [`completions`], and the Ctrl+E picker browses through [`search`]. Keeping
//! them on one table means a name that completes is a name that renders.
//!
//! Every glyph here is a single scalar (optionally plus U+FE0F) — no ZWJ
//! sequences. A terminal's idea of how wide `👨‍💻` is disagrees with
//! `unicode-width`'s often enough to shear the layout, and a chat client that
//! smears its own member panel is worse than one with fewer emoji.

/// One catalogue entry.
pub struct Emoji {
    /// The `:shortcode:` name, without colons. Unique across the table.
    pub name: &'static str,
    /// The character(s) to render.
    pub glyph: &'static str,
    /// Extra search terms for the picker. The name is always searched too, so
    /// these are for what a thing is *called by someone else*.
    pub keywords: &'static [&'static str],
}

/// Shorthand for a table row.
const fn e(
    name: &'static str,
    glyph: &'static str,
    keywords: &'static [&'static str],
) -> Emoji {
    Emoji {
        name,
        glyph,
        keywords,
    }
}

/// The catalogue. Ordered by category, because the picker shows this order
/// when the filter is empty and "the ones you actually react with" belongs at
/// the top far more than "aardvark" does.
pub static TABLE: &[Emoji] = &[
    // --- reactions and hands: the top of the list on purpose ---------------
    e("+1", "👍", &["thumbsup", "yes", "lgtm", "approve", "like"]),
    e("-1", "👎", &["thumbsdown", "no", "nope", "reject"]),
    e("ok_hand", "👌", &["perfect", "fine", "chef"]),
    e("clap", "👏", &["applause", "bravo", "well done"]),
    e("raised_hands", "🙌", &["celebrate", "praise", "hooray"]),
    e("pray", "🙏", &["thanks", "please", "thank you"]),
    e("wave", "👋", &["hello", "hi", "bye", "goodbye"]),
    e("muscle", "💪", &["strong", "flex", "power"]),
    e("handshake", "🤝", &["deal", "agree", "partner"]),
    e("fist", "✊", &["solidarity", "power"]),
    e("punch", "👊", &["bump", "fist bump"]),
    e("v", "✌️", &["peace", "victory"]),
    e("crossed_fingers", "🤞", &["luck", "hope", "fingers crossed"]),
    e("vulcan", "🖖", &["spock", "live long", "trek"]),
    e("call_me", "🤙", &["shaka", "hang loose"]),
    e("point_up", "☝️", &["up", "this", "attention"]),
    e("point_down", "👇", &["down", "below"]),
    e("point_left", "👈", &["left"]),
    e("point_right", "👉", &["right"]),
    e("raised_hand", "✋", &["stop", "halt", "highfive"]),
    e("open_hands", "👐", &["hug", "open"]),
    e("pinch", "🤏", &["small", "tiny", "little"]),
    e("writing_hand", "✍️", &["write", "sign", "note"]),
    e("salute", "🫡", &["yes sir", "respect", "o7"]),
    e("facepalm", "🤦", &["ugh", "no", "disbelief"]),
    e("shrug", "🤷", &["dunno", "idk", "whatever"]),
    // --- faces --------------------------------------------------------------
    e("smile", "😄", &["happy", "grin", "glad"]),
    e("grin", "😁", &["happy", "smile", "teeth"]),
    e("joy", "😂", &["lol", "laugh", "tears", "funny"]),
    e("rofl", "🤣", &["lmao", "laugh", "rolling"]),
    e("sweat_smile", "😅", &["phew", "relief", "nervous"]),
    e("wink", "😉", &["flirt", "joke"]),
    e("blush", "😊", &["shy", "happy", "smile"]),
    e("heart_eyes", "😍", &["love", "adore", "crush"]),
    e("kissing_heart", "😘", &["kiss", "love"]),
    e("thinking", "🤔", &["hmm", "consider", "ponder", "doubt"]),
    e("neutral", "😐", &["meh", "blank", "poker"]),
    e("expressionless", "😑", &["blank", "done", "unimpressed"]),
    e("no_mouth", "😶", &["silence", "speechless", "quiet"]),
    e("smirk", "😏", &["smug", "sly", "knowing"]),
    e("unamused", "😒", &["meh", "annoyed", "side eye"]),
    e("roll_eyes", "🙄", &["whatever", "annoyed", "ugh"]),
    e("grimace", "😬", &["yikes", "awkward", "oops"]),
    e("relieved", "😌", &["calm", "phew", "content"]),
    e("pensive", "😔", &["sad", "down", "regret"]),
    e("sleepy", "😪", &["tired", "drowsy"]),
    e("sleeping", "😴", &["asleep", "zzz", "bed"]),
    e("mask", "😷", &["sick", "ill", "flu"]),
    e("nerd", "🤓", &["geek", "glasses", "smart"]),
    e("sunglasses", "😎", &["cool", "deal with it", "shades"]),
    e("star_struck", "🤩", &["amazed", "wow", "excited"]),
    e("partying", "🥳", &["celebrate", "party", "birthday"]),
    e("upside_down", "🙃", &["irony", "sarcasm", "silly"]),
    e("money_mouth", "🤑", &["rich", "cash", "profit"]),
    e("hushed", "😯", &["surprise", "oh", "quiet"]),
    e("astonished", "😲", &["shocked", "wow", "gasp"]),
    e("scream", "😱", &["horror", "fear", "panic"]),
    e("fearful", "😨", &["scared", "worried"]),
    e("cold_sweat", "😰", &["anxious", "nervous", "stress"]),
    e("cry", "😢", &["sad", "tear", "upset"]),
    e("sob", "😭", &["crying", "bawling", "devastated"]),
    e("triumph", "😤", &["proud", "huff", "determined"]),
    e("angry", "😠", &["mad", "annoyed"]),
    e("rage", "😡", &["furious", "mad", "anger"]),
    e("exploding_head", "🤯", &["mind blown", "shocked", "wow"]),
    e("hot_face", "🥵", &["heat", "sweating", "overheated"]),
    e("cold_face", "🥶", &["freezing", "frozen", "ice"]),
    e("woozy", "🥴", &["drunk", "dizzy", "tipsy"]),
    e("dizzy_face", "😵", &["knocked out", "confused", "dead"]),
    e("zipper_mouth", "🤐", &["secret", "quiet", "shh"]),
    e("lying", "🤥", &["pinocchio", "liar", "false"]),
    e("shush", "🤫", &["quiet", "secret", "hush"]),
    e("yawn", "🥱", &["bored", "tired", "sleepy"]),
    e("melting", "🫠", &["dissolve", "hot", "embarrassed"]),
    e("clown", "🤡", &["joker", "fool", "circus"]),
    e("alien", "👽", &["ufo", "extraterrestrial", "space"]),
    e("ghost", "👻", &["boo", "spooky", "halloween"]),
    e("skull", "💀", &["dead", "rip", "death", "rekt"]),
    e("poop", "💩", &["crap", "bad", "garbage"]),
    e("robot", "🤖", &["bot", "ai", "android", "machine"]),
    e("ogre", "👹", &["demon", "monster", "oni"]),
    // --- machines and tooling ----------------------------------------------
    e("computer", "💻", &["laptop", "dev", "code", "work"]),
    e("desktop", "🖥️", &["pc", "monitor", "workstation"]),
    e("keyboard", "⌨️", &["type", "input", "keys"]),
    e("trackball", "🖱️", &["mouse", "pointer", "click"]),
    e("printer", "🖨️", &["print", "paper"]),
    e("floppy", "💾", &["save", "disk", "retro"]),
    e("cd", "💿", &["disc", "optical"]),
    e("dvd", "📀", &["disc", "optical"]),
    e("phone", "📱", &["mobile", "cell", "smartphone"]),
    e("telephone", "☎️", &["call", "landline"]),
    e("satellite", "📡", &["antenna", "signal", "uplink", "dish"]),
    e("battery", "🔋", &["power", "charge", "energy"]),
    e("plug", "🔌", &["power", "electric", "connect"]),
    e("bulb", "💡", &["idea", "light", "insight"]),
    e("flashlight", "🔦", &["torch", "light", "search"]),
    e("gear", "⚙️", &["settings", "config", "cog", "machine"]),
    e("wrench", "🔧", &["fix", "tool", "repair"]),
    e("hammer", "🔨", &["build", "tool", "smash"]),
    e("toolbox", "🧰", &["tools", "kit", "repair"]),
    e("screwdriver", "🪛", &["fix", "tool"]),
    e("nut_and_bolt", "🔩", &["hardware", "fasten"]),
    e("magnet", "🧲", &["attract", "pull"]),
    e("microscope", "🔬", &["science", "lab", "inspect", "research"]),
    e("telescope", "🔭", &["space", "observe", "astronomy"]),
    e("test_tube", "🧪", &["lab", "science", "experiment"]),
    e("dna", "🧬", &["genetics", "biology", "science"]),
    e("magnifying_glass", "🔍", &["search", "find", "look", "zoom"]),
    e("abacus", "🧮", &["count", "math", "calculate"]),
    e("joystick", "🕹️", &["game", "arcade", "retro"]),
    e("video_game", "🎮", &["gaming", "controller", "play"]),
    e("camera", "📷", &["photo", "picture", "screenshot"]),
    e("video_camera", "📹", &["record", "film", "video"]),
    e("clapper", "🎬", &["film", "movie", "action", "shoot"]),
    e("headphones", "🎧", &["audio", "music", "listen"]),
    e("microphone", "🎤", &["mic", "sing", "speak", "podcast"]),
    e("speaker", "🔊", &["loud", "volume", "audio"]),
    e("mute", "🔇", &["silent", "quiet", "volume off"]),
    e("bell", "🔔", &["notify", "alert", "ring"]),
    e("no_bell", "🔕", &["mute", "silence", "dnd"]),
    e("loudspeaker", "📢", &["announce", "broadcast", "shout"]),
    e("megaphone", "📣", &["announce", "cheer", "promote"]),
    // --- bugs, security, danger --------------------------------------------
    e("bug", "🐛", &["defect", "issue", "error", "caterpillar"]),
    e("spider", "🕷️", &["crawl", "web", "creepy"]),
    e("ant", "🐜", &["insect", "small"]),
    e("microbe", "🦠", &["virus", "germ", "bacteria"]),
    e("lock", "🔒", &["secure", "private", "closed", "encrypted"]),
    e("unlock", "🔓", &["open", "insecure", "access granted"]),
    e("key", "🔑", &["password", "secret", "access", "auth"]),
    e("old_key", "🗝️", &["secret", "legacy", "access"]),
    e("shield", "🛡️", &["defend", "protect", "security"]),
    e("crossed_swords", "⚔️", &["fight", "battle", "conflict"]),
    e("dagger", "🗡️", &["knife", "stab", "blade"]),
    e("gun", "🔫", &["shoot", "water pistol", "bang"]),
    e("bomb", "💣", &["explode", "danger", "boom"]),
    e("knife", "🔪", &["cut", "blade", "chop"]),
    e("radioactive", "☢️", &["nuclear", "nuke", "danger"]),
    e("biohazard", "☣️", &["toxic", "danger", "contamination"]),
    e("warning", "⚠️", &["caution", "alert", "careful"]),
    e("no_entry", "⛔", &["forbidden", "blocked", "denied"]),
    e("stop_sign", "🛑", &["halt", "stop", "block"]),
    e("sos", "🆘", &["help", "emergency", "mayday"]),
    // --- verdicts and marks -------------------------------------------------
    e("check", "✅", &["done", "yes", "pass", "green", "merged"]),
    e("heavy_check", "✔️", &["done", "tick", "yes"]),
    e("ballot_check", "☑️", &["checkbox", "done", "todo"]),
    e("x", "❌", &["no", "fail", "wrong", "red", "reject"]),
    e("cross_mark", "❎", &["no", "fail", "cancel"]),
    e("question", "❓", &["ask", "help", "unknown", "why"]),
    e("exclamation", "❗", &["alert", "important", "warning"]),
    e("bangbang", "‼️", &["urgent", "important", "very"]),
    e("interrobang", "⁉️", &["what", "confused", "surprise"]),
    e("info", "ℹ️", &["information", "note", "fyi"]),
    e("recycle", "♻️", &["reuse", "refactor", "green"]),
    e("new", "🆕", &["fresh", "latest"]),
    e("free", "🆓", &["gratis", "no cost"]),
    e("cool_badge", "🆒", &["nice", "ok"]),
    e("ok_badge", "🆗", &["fine", "agreed"]),
    e("top", "🔝", &["best", "up", "highest"]),
    e("soon", "🔜", &["later", "wip", "coming"]),
    e("hundred", "💯", &["perfect", "score", "full", "agree"]),
    // --- energy, weather, sky ----------------------------------------------
    e("fire", "🔥", &["hot", "lit", "burn", "flame", "shipping"]),
    e("zap", "⚡", &["lightning", "fast", "power", "electric"]),
    e("boom", "💥", &["explosion", "collision", "headshot", "crash"]),
    e("sparkles", "✨", &["shiny", "new", "magic", "clean"]),
    e("star", "⭐", &["favourite", "rating", "starred"]),
    e("glowing_star", "🌟", &["shine", "special", "bright"]),
    e("dizzy", "💫", &["stars", "spin", "sparkle"]),
    e("comet", "☄️", &["meteor", "space", "fast"]),
    e("rainbow", "🌈", &["colour", "pride", "arc"]),
    e("snowflake", "❄️", &["cold", "winter", "freeze", "flaky"]),
    e("droplet", "💧", &["water", "drop", "liquid"]),
    e("sweat_drops", "💦", &["water", "splash", "effort"]),
    e("ocean", "🌊", &["wave", "sea", "water"]),
    e("tornado", "🌪️", &["storm", "chaos", "twister"]),
    e("fog", "🌫️", &["mist", "unclear", "haze"]),
    e("cloud", "☁️", &["sky", "server", "weather"]),
    e("sun", "☀️", &["sunny", "day", "bright", "clear"]),
    e("moon", "🌙", &["night", "crescent", "dark"]),
    e("earth", "🌍", &["world", "globe", "planet"]),
    e("milky_way", "🌌", &["space", "stars", "galaxy"]),
    e("volcano", "🌋", &["eruption", "lava", "danger"]),
    // --- motion -------------------------------------------------------------
    e("rocket", "🚀", &["ship", "launch", "deploy", "fast", "release"]),
    e("ufo", "🛸", &["alien", "saucer", "space"]),
    e("airplane", "✈️", &["flight", "travel", "fly"]),
    e("helicopter", "🚁", &["chopper", "fly"]),
    e("parachute", "🪂", &["jump", "fall", "safe"]),
    e("car", "🚗", &["drive", "auto", "vehicle"]),
    e("taxi", "🚕", &["cab", "ride"]),
    e("bus", "🚌", &["transit", "ride"]),
    e("truck", "🚚", &["delivery", "shipping", "haul"]),
    e("tractor", "🚜", &["farm", "slow"]),
    e("ambulance", "🚑", &["emergency", "medical", "hospital"]),
    e("fire_engine", "🚒", &["firetruck", "emergency", "incident"]),
    e("police_car", "🚓", &["cop", "law", "siren"]),
    e("train", "🚆", &["rail", "transit"]),
    e("ship", "🚢", &["boat", "sail", "deploy"]),
    e("anchor", "⚓", &["harbour", "stable", "dock"]),
    e("bike", "🚲", &["cycle", "ride"]),
    e("motorcycle", "🏍️", &["ride", "moto", "fast"]),
    // --- animals ------------------------------------------------------------
    e("penguin", "🐧", &["linux", "tux", "bird"]),
    e("cat", "🐱", &["kitten", "meow", "feline"]),
    e("dog", "🐶", &["puppy", "woof", "canine"]),
    e("fox", "🦊", &["firefox", "sly"]),
    e("bear", "🐻", &["grizzly"]),
    e("panda", "🐼", &["bamboo", "bear"]),
    e("koala", "🐨", &["australia", "bear"]),
    e("tiger", "🐯", &["cat", "stripes"]),
    e("lion", "🦁", &["king", "roar"]),
    e("wolf", "🐺", &["howl", "pack"]),
    e("cow", "🐮", &["moo", "cattle"]),
    e("pig", "🐷", &["oink", "hog"]),
    e("frog", "🐸", &["ribbit", "toad"]),
    e("monkey", "🐵", &["ape", "primate"]),
    e("see_no_evil", "🙈", &["monkey", "hide", "oops"]),
    e("hear_no_evil", "🙉", &["monkey", "ignore"]),
    e("speak_no_evil", "🙊", &["monkey", "secret", "quiet"]),
    e("chicken", "🐔", &["hen", "bird", "cluck"]),
    e("unicorn", "🦄", &["magic", "rare", "startup"]),
    e("horse", "🐴", &["pony", "neigh"]),
    e("whale", "🐳", &["docker", "ocean", "big"]),
    e("dolphin", "🐬", &["sea", "smart"]),
    e("fish", "🐟", &["sea", "trout", "slap"]),
    e("shark", "🦈", &["jaws", "danger", "sea"]),
    e("octopus", "🐙", &["tentacles", "git", "sea"]),
    e("crab", "🦀", &["rust", "ferris", "claw"]),
    e("snake", "🐍", &["python", "hiss", "serpent"]),
    e("turtle", "🐢", &["slow", "shell", "tortoise"]),
    e("dragon", "🐉", &["fire", "myth", "beast"]),
    e("butterfly", "🦋", &["change", "pretty", "insect"]),
    e("bee", "🐝", &["honey", "buzz", "busy"]),
    e("snail", "🐌", &["slow", "lag"]),
    e("owl", "🦉", &["night", "wise", "bird"]),
    e("eagle", "🦅", &["bird", "fly", "freedom"]),
    e("bat", "🦇", &["night", "vampire", "cave"]),
    e("rabbit", "🐰", &["bunny", "fast", "hop"]),
    e("mouse", "🐭", &["rodent", "small", "squeak"]),
    e("hamster", "🐹", &["pet", "rodent", "wheel"]),
    e("sheep", "🐑", &["wool", "baa", "flock"]),
    e("elephant", "🐘", &["big", "memory", "postgres"]),
    e("giraffe", "🦒", &["tall", "neck"]),
    e("sloth", "🦥", &["slow", "lazy", "tired"]),
    e("hedgehog", "🦔", &["spiky", "small"]),
    // --- fuel ---------------------------------------------------------------
    e("coffee", "☕", &["caffeine", "espresso", "morning", "brew"]),
    e("tea", "🍵", &["green tea", "matcha", "brew"]),
    e("bubble_tea", "🧋", &["boba", "milk tea"]),
    e("beer", "🍺", &["pint", "drink", "pub"]),
    e("beers", "🍻", &["cheers", "toast", "drink"]),
    e("wine", "🍷", &["red", "drink", "glass"]),
    e("cocktail", "🍸", &["martini", "drink", "bar"]),
    e("tropical_drink", "🍹", &["holiday", "cocktail", "beach"]),
    e("champagne", "🍾", &["celebrate", "pop", "launch"]),
    e("whisky", "🥃", &["whiskey", "scotch", "drink"]),
    e("milk", "🥛", &["dairy", "glass"]),
    e("pizza", "🍕", &["slice", "food", "italian"]),
    e("hamburger", "🍔", &["burger", "food", "fast food"]),
    e("fries", "🍟", &["chips", "food", "fast food"]),
    e("hotdog", "🌭", &["sausage", "food"]),
    e("taco", "🌮", &["mexican", "food"]),
    e("burrito", "🌯", &["wrap", "mexican", "food"]),
    e("sushi", "🍣", &["japanese", "fish", "food"]),
    e("ramen", "🍜", &["noodles", "soup", "japanese"]),
    e("spaghetti", "🍝", &["pasta", "italian", "code"]),
    e("curry", "🍛", &["spicy", "rice", "food"]),
    e("rice", "🍚", &["bowl", "staple"]),
    e("bread", "🍞", &["loaf", "toast", "bakery"]),
    e("croissant", "🥐", &["pastry", "french", "bakery"]),
    e("cheese", "🧀", &["dairy", "cheddar"]),
    e("egg", "🥚", &["breakfast", "easter"]),
    e("bacon", "🥓", &["breakfast", "pork"]),
    e("popcorn", "🍿", &["movie", "drama", "watching"]),
    e("doughnut", "🍩", &["donut", "sweet", "sugar"]),
    e("cookie", "🍪", &["biscuit", "sweet", "cache"]),
    e("cake", "🍰", &["slice", "sweet", "dessert"]),
    e("birthday", "🎂", &["cake", "celebrate", "anniversary"]),
    e("chocolate", "🍫", &["sweet", "candy", "cocoa"]),
    e("candy", "🍬", &["sweet", "sugar"]),
    e("lollipop", "🍭", &["sweet", "candy"]),
    e("ice_cream", "🍦", &["dessert", "cold", "sweet"]),
    e("watermelon", "🍉", &["fruit", "summer"]),
    e("apple", "🍎", &["fruit", "mac", "red"]),
    e("banana", "🍌", &["fruit", "yellow"]),
    e("grapes", "🍇", &["fruit", "wine"]),
    e("strawberry", "🍓", &["fruit", "berry", "red"]),
    e("peach", "🍑", &["fruit", "summer"]),
    e("cherries", "🍒", &["fruit", "red"]),
    e("pineapple", "🍍", &["fruit", "tropical"]),
    e("mango", "🥭", &["fruit", "tropical"]),
    e("avocado", "🥑", &["fruit", "toast", "green"]),
    e("carrot", "🥕", &["vegetable", "orange"]),
    e("corn", "🌽", &["maize", "vegetable"]),
    e("hot_pepper", "🌶️", &["spicy", "chilli", "heat"]),
    e("broccoli", "🥦", &["vegetable", "green", "healthy"]),
    e("mushroom", "🍄", &["fungus", "toadstool"]),
    e("peanuts", "🥜", &["nuts", "snack"]),
    e("honey", "🍯", &["sweet", "bee", "pot"]),
    e("salt", "🧂", &["salty", "seasoning"]),
    // --- objects and occasions ---------------------------------------------
    e("package", "📦", &["box", "delivery", "release", "npm", "crate"]),
    e("gift", "🎁", &["present", "birthday", "surprise"]),
    e("balloon", "🎈", &["party", "celebrate"]),
    e("tada", "🎉", &["party", "celebrate", "hooray", "launch", "ship"]),
    e("confetti", "🎊", &["party", "celebrate"]),
    e("trophy", "🏆", &["win", "award", "champion", "first"]),
    e("medal", "🏅", &["award", "win", "honour"]),
    e("first_place", "🥇", &["gold", "win", "best"]),
    e("second_place", "🥈", &["silver", "runner up"]),
    e("third_place", "🥉", &["bronze", "third"]),
    e("dart", "🎯", &["target", "bullseye", "goal", "aim"]),
    e("game_die", "🎲", &["dice", "random", "chance", "luck"]),
    e("jigsaw", "🧩", &["puzzle", "piece", "plugin", "fit"]),
    e("teddy", "🧸", &["bear", "toy", "soft"]),
    e("crystal_ball", "🔮", &["future", "predict", "magic"]),
    e("magic_wand", "🪄", &["magic", "spell", "fix"]),
    e("pill", "💊", &["medicine", "red pill", "blue pill", "matrix"]),
    e("syringe", "💉", &["injection", "vaccine", "inject"]),
    e("gem", "💎", &["diamond", "ruby", "precious", "valuable"]),
    e("ring", "💍", &["marriage", "engaged", "jewellery"]),
    e("crown", "👑", &["king", "queen", "royal", "admin"]),
    e("tophat", "🎩", &["magic", "formal", "hat"]),
    e("dark_sunglasses", "🕶️", &["shades", "cool", "incognito"]),
    e("money", "💰", &["cash", "bag", "rich", "budget"]),
    e("dollar", "💵", &["cash", "money", "usd"]),
    e("credit_card", "💳", &["payment", "money", "billing"]),
    e("receipt", "🧾", &["invoice", "bill", "expense"]),
    e("coin", "🪙", &["money", "token", "crypto"]),
    // --- paper, office, data ------------------------------------------------
    e("book", "📖", &["read", "docs", "manual", "open book"]),
    e("books", "📚", &["library", "docs", "study"]),
    e("notebook", "📓", &["notes", "journal"]),
    e("page", "📄", &["document", "file", "text"]),
    e("scroll", "📜", &["document", "ancient", "licence"]),
    e("bookmark", "🔖", &["save", "tag", "mark"]),
    e("label", "🏷️", &["tag", "name", "price"]),
    e("clipboard", "📋", &["copy", "paste", "list", "todo"]),
    e("pushpin", "📌", &["pin", "sticky", "important"]),
    e("paperclip", "📎", &["attach", "file", "clip"]),
    e("straight_ruler", "📏", &["measure", "length", "size"]),
    e("triangular_ruler", "📐", &["measure", "angle", "design"]),
    e("scissors", "✂️", &["cut", "snip", "trim"]),
    e("pen", "🖊️", &["write", "sign", "ink"]),
    e("pencil", "✏️", &["write", "edit", "draft"]),
    e("paintbrush", "🖌️", &["paint", "art", "style"]),
    e("art", "🎨", &["palette", "design", "colour", "creative"]),
    e("envelope", "✉️", &["mail", "letter", "message"]),
    e("email", "📧", &["mail", "inbox", "message"]),
    e("inbox", "📥", &["receive", "incoming", "download"]),
    e("outbox", "📤", &["send", "outgoing", "upload"]),
    e("mailbox", "📫", &["post", "mail", "letters"]),
    e("calendar", "📅", &["date", "schedule", "plan"]),
    e("chart", "📊", &["bar chart", "data", "stats", "metrics"]),
    e("chart_up", "📈", &["growth", "increase", "trending", "win"]),
    e("chart_down", "📉", &["decline", "decrease", "loss", "regression"]),
    e("file_folder", "📁", &["directory", "files", "folder"]),
    e("open_folder", "📂", &["directory", "browse", "open"]),
    e("card_index", "🗂️", &["organise", "dividers", "index"]),
    e("wastebasket", "🗑️", &["trash", "delete", "bin", "remove"]),
    // --- time ---------------------------------------------------------------
    e("hourglass", "⏳", &["wait", "time", "pending", "loading"]),
    e("hourglass_done", "⌛", &["finished", "time up", "expired"]),
    e("watch", "⌚", &["time", "clock", "wrist"]),
    e("alarm_clock", "⏰", &["wake", "alert", "reminder", "deadline"]),
    e("stopwatch", "⏱️", &["timer", "benchmark", "speed", "latency"]),
    e("timer", "⏲️", &["countdown", "timeout"]),
    e("clock", "🕐", &["time", "hour", "schedule"]),
    // --- hearts and marks ---------------------------------------------------
    e("heart", "❤️", &["love", "red", "like"]),
    e("orange_heart", "🧡", &["love", "orange"]),
    e("yellow_heart", "💛", &["love", "yellow"]),
    e("green_heart", "💚", &["love", "green"]),
    e("blue_heart", "💙", &["love", "blue"]),
    e("purple_heart", "💜", &["love", "purple"]),
    e("black_heart", "🖤", &["love", "dark", "goth"]),
    e("white_heart", "🤍", &["love", "pure"]),
    e("broken_heart", "💔", &["sad", "breakup", "hurt"]),
    e("two_hearts", "💕", &["love", "affection"]),
    e("sparkling_heart", "💖", &["love", "shiny"]),
    e("heartbeat", "💓", &["pulse", "love", "alive"]),
    e("anger", "💢", &["mad", "vein", "annoyed"]),
    e("zzz", "💤", &["sleep", "idle", "afk", "tired"]),
    e("speech_balloon", "💬", &["chat", "talk", "comment", "message"]),
    e("thought_balloon", "💭", &["think", "idea", "dream"]),
    e("eye", "👁️", &["watch", "see", "look"]),
    e("eyes", "👀", &["looking", "watching", "review", "peek"]),
    e("brain", "🧠", &["think", "smart", "mind", "logic"]),
    e("footprints", "👣", &["steps", "trace", "follow"]),
    e("infinity", "♾️", &["forever", "endless", "loop"]),
    e("peace", "☮️", &["calm", "hippie", "no war"]),
    e("yin_yang", "☯️", &["balance", "harmony", "tao"]),
    e("atom", "⚛️", &["react", "science", "physics", "nuclear"]),
    e("trident", "🔱", &["neptune", "fork", "spear"]),
    e("copyright", "©️", &["legal", "rights"]),
    e("registered", "®️", &["trademark", "legal"]),
    e("tm", "™️", &["trademark", "brand"]),
    // --- arrows and transport controls --------------------------------------
    e("arrow_up", "⬆️", &["north", "increase", "up"]),
    e("arrow_down", "⬇️", &["south", "decrease", "down"]),
    e("arrow_left", "⬅️", &["west", "back", "left"]),
    e("arrow_right", "➡️", &["east", "forward", "next"]),
    e("refresh", "🔄", &["reload", "sync", "retry", "again"]),
    e("repeat", "🔁", &["loop", "again", "cycle"]),
    e("repeat_one", "🔂", &["loop one", "single"]),
    e("shuffle", "🔀", &["random", "mix", "swap"]),
    e("play", "▶️", &["start", "run", "go"]),
    e("pause", "⏸️", &["hold", "wait", "stop"]),
    e("stop_button", "⏹️", &["halt", "end", "kill"]),
    e("record", "⏺️", &["capture", "rec"]),
    e("fast_forward", "⏩", &["skip", "speed", "next"]),
    e("rewind", "⏪", &["back", "replay", "undo"]),
    e("plus", "➕", &["add", "more", "increment"]),
    e("minus", "➖", &["subtract", "less", "decrement"]),
    e("divide", "➗", &["division", "split"]),
    e("multiply", "✖️", &["times", "product"]),
    // --- places and nature --------------------------------------------------
    e("house", "🏠", &["home", "building", "local"]),
    e("office", "🏢", &["work", "building", "company"]),
    e("factory", "🏭", &["industry", "production", "build"]),
    e("hospital", "🏥", &["medical", "health", "emergency"]),
    e("bank", "🏦", &["money", "finance", "building"]),
    e("school", "🏫", &["education", "learn", "building"]),
    e("castle", "🏰", &["fortress", "royal", "medieval"]),
    e("tent", "⛺", &["camp", "outdoors", "temporary"]),
    e("mountain", "⛰️", &["peak", "climb", "summit"]),
    e("desert", "🏜️", &["dry", "sand", "empty"]),
    e("beach", "🏖️", &["holiday", "sand", "sea"]),
    e("island", "🏝️", &["tropical", "alone", "holiday"]),
    e("city", "🏙️", &["skyline", "urban", "buildings"]),
    e("night_city", "🌃", &["skyline", "dark", "urban"]),
    e("sunrise", "🌅", &["morning", "dawn", "start"]),
    e("sunset", "🌇", &["evening", "dusk", "end"]),
    e("bridge", "🌉", &["connect", "span", "night"]),
    e("tree", "🌳", &["nature", "green", "branch"]),
    e("palm_tree", "🌴", &["tropical", "holiday", "beach"]),
    e("cactus", "🌵", &["desert", "spiky", "dry"]),
    e("herb", "🌿", &["plant", "green", "leaf"]),
    e("four_leaf_clover", "🍀", &["luck", "lucky", "irish"]),
    e("maple_leaf", "🍁", &["autumn", "canada", "fall"]),
    e("fallen_leaf", "🍂", &["autumn", "fall", "dry"]),
    e("leaves", "🍃", &["wind", "green", "nature"]),
    e("seedling", "🌱", &["grow", "new", "sprout", "start"]),
    e("blossom", "🌸", &["flower", "spring", "sakura"]),
    e("rose", "🌹", &["flower", "love", "red"]),
    e("sunflower", "🌻", &["flower", "yellow", "sun"]),
    e("tulip", "🌷", &["flower", "spring"]),
    e("bouquet", "💐", &["flowers", "gift", "thanks"]),
    // --- flags and characters -----------------------------------------------
    e("checkered_flag", "🏁", &["finish", "race", "done", "start"]),
    e("triangular_flag", "🚩", &["flag", "mark", "warning", "report"]),
    e("white_flag", "🏳️", &["surrender", "give up", "truce"]),
    e("black_flag", "🏴", &["pirate", "anarchy"]),
    e("ninja", "🥷", &["stealth", "hidden", "assassin"]),
    e("detective", "🕵️", &["spy", "investigate", "sleuth"]),
    e("mage", "🧙", &["wizard", "magic", "spell"]),
    e("vampire", "🧛", &["dracula", "night", "blood"]),
    e("zombie", "🧟", &["undead", "walking dead", "brains"]),
    e("genie", "🧞", &["wish", "lamp", "magic"]),
    e("superhero", "🦸", &["hero", "save", "cape"]),
    e("santa", "🎅", &["christmas", "xmas", "gifts"]),
];

/// Looks up a `:shortcode:` name.
#[must_use]
pub fn find(name: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.glyph)
}

/// Tab-completion candidates for a partial name, already wrapped in colons so
/// the generic completer can match and insert them verbatim.
///
/// Prefix matching on the name only — deliberately narrower than [`search`].
/// Tab is for a name you half-remember; the picker is for one you don't.
#[must_use]
pub fn completions(stem: &str) -> Vec<String> {
    // A bare `:` would offer the whole catalogue, which is a picker's job.
    if stem.is_empty() {
        return Vec::new();
    }
    let stem = stem.to_lowercase();
    TABLE
        .iter()
        .filter(|entry| entry.name.starts_with(&stem))
        .map(|entry| format!(":{}:", entry.name))
        .collect()
}

/// Picker results for a query, best match first. An empty query is the whole
/// catalogue in table order.
///
/// Ranking runs name-before-keyword and exact-before-prefix-before-substring,
/// so typing `fire` puts 🔥 ahead of 🚒 without any fuzzy-match machinery.
#[must_use]
pub fn search(query: &str) -> Vec<&'static Emoji> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return TABLE.iter().collect();
    }

    let mut ranked: Vec<(u8, &'static Emoji)> = TABLE
        .iter()
        .filter_map(|entry| {
            let rank = if entry.name == query {
                0
            } else if entry.name.starts_with(&query) {
                1
            } else if entry.keywords.iter().any(|k| *k == query) {
                2
            } else if entry.keywords.iter().any(|k| k.starts_with(&query)) {
                3
            } else if entry.name.contains(&query) {
                4
            } else if entry.keywords.iter().any(|k| k.contains(&query)) {
                5
            } else {
                return None;
            };
            Some((rank, entry))
        })
        .collect();

    // Stable within a rank, so equally-good matches keep catalogue order.
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique() {
        let mut seen = HashSet::new();
        for entry in TABLE {
            assert!(seen.insert(entry.name), "duplicate name :{}:", entry.name);
        }
    }

    #[test]
    fn names_are_shortcode_shaped() {
        for entry in TABLE {
            assert!(!entry.name.is_empty(), "empty name");
            assert!(
                entry
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '+'
                        || c == '-'),
                "name :{}: has characters that cannot be typed as a shortcode",
                entry.name
            );
            // A colon inside a name would make expand_emoji unable to find it.
            assert!(!entry.name.contains(':'), "name :{}: has a colon", entry.name);
        }
    }

    #[test]
    fn glyphs_carry_no_zwj_sequences() {
        // ZWJ emoji render at widths unicode-width cannot predict, which
        // shears the message pane. Keep them out of the table entirely.
        for entry in TABLE {
            assert!(
                !entry.glyph.contains('\u{200d}'),
                ":{}: is a ZWJ sequence",
                entry.name
            );
            assert!(!entry.glyph.is_empty(), ":{}: has no glyph", entry.name);
        }
    }

    #[test]
    fn the_original_shortcodes_still_resolve() {
        // Messages already on servers use these. Changing one silently
        // rewrites history, so they are pinned.
        let legacy = [
            ("fire", "🔥"),
            ("rocket", "🚀"),
            ("computer", "💻"),
            ("penguin", "🐧"),
            ("zap", "⚡"),
            ("tada", "🎉"),
            ("heart", "❤️"),
            ("+1", "👍"),
            ("-1", "👎"),
            ("smile", "😄"),
            ("grin", "😁"),
            ("joy", "😂"),
            ("eyes", "👀"),
            ("thinking", "🤔"),
            ("warning", "⚠️"),
            ("lock", "🔒"),
            ("key", "🔑"),
            ("skull", "💀"),
            ("ghost", "👻"),
            ("robot", "🤖"),
            ("coffee", "☕"),
            ("bug", "🐛"),
            ("check", "✅"),
            ("x", "❌"),
        ];
        for (name, glyph) in legacy {
            assert_eq!(find(name), Some(glyph), ":{name}: changed or vanished");
        }
    }

    #[test]
    fn completions_are_prefix_matches_wrapped_in_colons() {
        let hits = completions("roc");
        assert!(hits.contains(&":rocket:".to_owned()));
        assert!(hits.iter().all(|h| h.starts_with(':') && h.ends_with(':')));

        // Case-insensitive, and unknown stems complete to nothing.
        assert!(completions("ROC").contains(&":rocket:".to_owned()));
        assert!(completions("zzzzznope").is_empty());

        // A bare colon defers to the picker rather than dumping the catalogue.
        assert!(completions("").is_empty());
    }

    #[test]
    fn search_ranks_the_obvious_match_first() {
        // Exact name beats the fire_engine/fire-keyword crowd.
        assert_eq!(search("fire")[0].name, "fire");
        // Keyword-only match still lands.
        assert!(search("lgtm").iter().any(|e| e.name == "+1"));
        assert!(search("docker").iter().any(|e| e.name == "whale"));
        // Nothing matches nothing.
        assert!(search("qqqqzzz").is_empty());
        // Empty query is the whole catalogue.
        assert_eq!(search("  ").len(), TABLE.len());
    }

    #[test]
    fn the_searches_the_readme_advertises_still_work() {
        // The README shows this exact result row for `fire`. If a table edit
        // reorders it, the screenshot in the docs became a lie — fix one or
        // the other.
        let names: Vec<&str> = search("fire").iter().map(|e| e.name).collect();
        assert_eq!(names, ["fire", "fire_engine", "dragon", "fox"]);

        // And the four "arrive from the word you thought of" examples.
        for (query, expected) in [
            ("lgtm", "+1"),
            ("docker", "whale"),
            ("deploy", "rocket"),
            ("flaky", "snowflake"),
        ] {
            assert_eq!(
                search(query).first().map(|e| e.name),
                Some(expected),
                "README says {query:?} finds :{expected}:"
            );
        }
    }
}
