//! Russian grammar the context model can check across several words: a
//! reading's grammemes as bits, the cases prepositions govern, the persons of
//! personal pronouns — and how well a word fits the phrase before it
//! ([`misfit`]): a noun phrase after a preposition takes its case, its
//! words agree in case, number and gender, a verb agrees with its pronoun.

/// A reading's grammemes (OpenCorpora names, as pymorphy3 gives them).
pub mod bit {
    pub const NOUN: u64 = 1 << 0;
    pub const ADJF: u64 = 1 << 1;
    pub const ADJS: u64 = 1 << 2;
    pub const COMP: u64 = 1 << 3;
    pub const VERB: u64 = 1 << 4;
    pub const INFN: u64 = 1 << 5;
    pub const PRTF: u64 = 1 << 6;
    pub const PRTS: u64 = 1 << 7;
    pub const GRND: u64 = 1 << 8;
    pub const NUMR: u64 = 1 << 9;
    pub const ADVB: u64 = 1 << 10;
    pub const NPRO: u64 = 1 << 11;
    pub const PRED: u64 = 1 << 12;
    pub const PREP: u64 = 1 << 13;
    pub const CONJ: u64 = 1 << 14;
    pub const PRCL: u64 = 1 << 15;
    pub const INTJ: u64 = 1 << 16;

    pub const MASC: u64 = 1 << 20;
    pub const FEMN: u64 = 1 << 21;
    pub const NEUT: u64 = 1 << 22;
    /// Common gender (сирота, коллега).
    pub const MSF: u64 = 1 << 23;
    pub const SING: u64 = 1 << 24;
    pub const PLUR: u64 = 1 << 25;

    pub const NOMN: u64 = 1 << 28;
    pub const GENT: u64 = 1 << 29;
    pub const DATV: u64 = 1 << 30;
    pub const ACCS: u64 = 1 << 31;
    pub const ABLT: u64 = 1 << 32;
    pub const LOCT: u64 = 1 << 33;
    pub const VOCT: u64 = 1 << 34;
    pub const GEN2: u64 = 1 << 35;
    pub const ACC2: u64 = 1 << 36;
    pub const LOC2: u64 = 1 << 37;

    pub const PER1: u64 = 1 << 40;
    pub const PER2: u64 = 1 << 41;
    pub const PER3: u64 = 1 << 42;
    pub const PAST: u64 = 1 << 44;
    pub const PRES: u64 = 1 << 45;
    pub const FUTR: u64 = 1 << 46;

    pub const GENDER: u64 = MASC | FEMN | NEUT | MSF;
    pub const NUMBER: u64 = SING | PLUR;
    pub const CASE: u64 = NOMN | GENT | DATV | ACCS | ABLT | LOCT | VOCT | GEN2 | ACC2 | LOC2;
    pub const PERSON: u64 = PER1 | PER2 | PER3;
    /// Words of a noun phrase: they take its case (and agree).
    pub const NOMINAL: u64 = NOUN | ADJF | PRTF | NPRO | NUMR;
    /// Words that agree with the noun they go with.
    pub const ATTRIBUTE: u64 = ADJF | PRTF;
}

use bit::*;

/// The bits of a reading written as pymorphy3's grammemes, comma-separated
/// (`ADJF,masc,sing,nomn`); unknown ones are left out.
pub fn parse(tag: &str) -> u64 {
    tag.split([',', ' '])
        .map(|g| match g.trim() {
            "NOUN" => NOUN,
            "ADJF" => ADJF,
            "ADJS" => ADJS,
            "COMP" => COMP,
            "VERB" => VERB,
            "INFN" => INFN,
            "PRTF" => PRTF,
            "PRTS" => PRTS,
            "GRND" => GRND,
            "NUMR" => NUMR,
            "ADVB" => ADVB,
            "NPRO" => NPRO,
            "PRED" => PRED,
            "PREP" => PREP,
            "CONJ" => CONJ,
            "PRCL" => PRCL,
            "INTJ" => INTJ,
            "masc" => MASC,
            "femn" => FEMN,
            "neut" => NEUT,
            "ms-f" => MSF,
            "sing" => SING,
            "plur" => PLUR,
            "nomn" => NOMN,
            "gent" => GENT,
            "datv" => DATV,
            "accs" => ACCS,
            "ablt" => ABLT,
            "loct" => LOCT,
            "voct" => VOCT,
            "gen2" => GEN2,
            "acc2" => ACC2,
            "loc2" => LOC2,
            "1per" => PER1,
            "2per" => PER2,
            "3per" => PER3,
            "past" => PAST,
            "pres" => PRES,
            "futr" => FUTR,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

/// The case bits with the second genitive, accusative and locative (чаю, в
/// лесу) counted as the first.
fn cases(r: u64) -> u64 {
    let mut c = r & CASE;
    if c & GEN2 != 0 {
        c |= GENT;
    }
    if c & ACC2 != 0 {
        c |= ACCS;
    }
    if c & LOC2 != 0 {
        c |= LOCT;
    }
    c & (NOMN | GENT | DATV | ACCS | ABLT | LOCT | VOCT)
}

/// The cases a preposition governs (not «надо», «вроде», «сверху»: mostly
/// they are something else; nor the rare «с» with the accusative).
pub fn governs(word: &str) -> Option<u64> {
    Some(match word {
        "в" | "во" | "на" => ACCS | LOCT,
        "о" | "об" | "обо" => LOCT | ACCS,
        "при" => LOCT,
        "по" => DATV | LOCT | ACCS,
        "с" | "со" => ABLT | GENT,
        "к" | "ко" | "благодаря" | "согласно" | "вопреки" | "навстречу" | "подобно" => {
            DATV
        }
        "за" | "под" | "подо" => ACCS | ABLT,
        "над" | "перед" | "передо" => ABLT,
        "между" => ABLT | GENT,
        "через" | "сквозь" | "про" => ACCS,
        "у"
        | "из"
        | "изо"
        | "от"
        | "ото"
        | "до"
        | "без"
        | "безо"
        | "для"
        | "из-за"
        | "из-под"
        | "около"
        | "возле"
        | "вокруг"
        | "после"
        | "кроме"
        | "среди"
        | "вместо"
        | "мимо"
        | "против"
        | "ради"
        | "сверх"
        | "ввиду"
        | "вследствие"
        | "насчёт"
        | "насчет"
        | "позади"
        | "внутри"
        | "вне"
        | "близ"
        | "вдоль"
        | "напротив"
        | "прежде"
        | "посреди"
        | "относительно"
        | "накануне"
        | "помимо"
        | "внутрь"
        | "поверх" => GENT,
        _ => return None,
    })
}

/// A personal pronoun as a subject: its person, number and (third person
/// singular) gender.
fn subject(word: &str) -> Option<u64> {
    Some(match word {
        "я" => PER1 | SING,
        "ты" => PER2 | SING,
        "он" => PER3 | SING | MASC,
        "она" => PER3 | SING | FEMN,
        "оно" => PER3 | SING | NEUT,
        "мы" => PER1 | PLUR,
        "вы" => PER2 | PLUR,
        "они" => PER3 | PLUR,
        _ => return None,
    })
}

/// Two readings agree (an attribute and what it goes with): a case, the
/// number and, in the singular, the gender in common.
fn agree(a: u64, b: u64) -> bool {
    if cases(a) & cases(b) == 0 || a & b & NUMBER == 0 {
        return false;
    }
    let singular = a & b & SING != 0;
    let (ga, gb) = (a & GENDER, b & GENDER);
    !singular || ga == 0 || gb == 0 || ga & gb != 0 || (ga | gb) & MSF != 0
}

/// A verb reading fits its subject: past tense by number (and gender in the
/// singular of the third person), present and future by person and number,
/// the imperative (no tense) only «ты» and «вы».
fn fits_subject(verb: u64, subj: u64) -> bool {
    if verb & subj & NUMBER == 0 {
        return false;
    }
    if verb & (PAST | PRES | FUTR) == 0 {
        return subj & PER2 != 0;
    }
    if verb & PAST != 0 {
        let g = subj & GENDER;
        return verb & SING == 0 || g == 0 || verb & g != 0;
    }
    verb & PERSON == 0 || verb & subj & PERSON != 0
}

/// Whether `word` (its readings) can't stand after the phrase before it —
/// `phrase`, the words before it in order, each with its readings: a noun
/// phrase after a preposition in a case it doesn't govern, a word that
/// doesn't agree with the adjectives before it, a verb that doesn't agree
/// with its subject pronoun. The grammar only rules out: what fits gains
/// nothing, since a preposition, a conjunction or an adverb may come as well
/// («один из», «могу я взять»); and only a word all of whose readings are
/// ruled out (every reading is kept, the rare ones too: «из» is also an
/// abbreviated noun).
pub fn misfit(phrase: &[(&str, Vec<u64>)], word: &[u64]) -> bool {
    if word.is_empty() {
        return false;
    }
    let attributive = |r: &Vec<u64>| r.iter().any(|x| x & ATTRIBUTE != 0 && x & CASE != 0);
    // Back from the word: attributes (and «и» between them) up to a
    // preposition; or a subject pronoun, maybe with adverbs and «не» between.
    let mut governed: Option<u64> = None;
    let mut attributes: Vec<&Vec<u64>> = Vec::new();
    let mut subj: Option<u64> = None;
    for (i, (w, readings)) in phrase.iter().rev().take(5).enumerate() {
        if let Some(c) = governs(w) {
            governed = Some(c);
            break;
        }
        // «который» starts a clause: the next word isn't its noun.
        if w.starts_with("котор") {
            break;
        }
        if i == attributes.len() && attributive(readings) {
            // The attributes of one phrase agree with each other: «к этому
            // никакого отношения» are two phrases.
            let next = attributes.iter().rev().find(|a| attributive(a));
            if next.is_some_and(|n| !readings.iter().any(|&x| n.iter().any(|&y| agree(x, y)))) {
                break;
            }
            attributes.push(readings);
            continue;
        }
        if !attributes.is_empty() && matches!(*w, "и" | "или") {
            attributes.push(readings);
            continue;
        }
        if attributes.is_empty() {
            if let Some(s) = subject(w) {
                subj = Some(s);
                break;
            }
            if readings.iter().all(|r| r & (ADVB | PRCL) != 0) && !readings.is_empty() {
                continue;
            }
        }
        break;
    }
    // A verb and its subject.
    if let Some(s) = subj {
        let verbs: Vec<u64> = word.iter().copied().filter(|r| r & VERB != 0).collect();
        return verbs.len() == word.len() && !verbs.iter().any(|&v| fits_subject(v, s));
    }
    // Only a word that can't be anything but an attribute asks for
    // agreement: «это», «его», «все» may be pronouns or a particle («это моя
    // книга»); they only carry the phrase on to its preposition.
    let attributes: Vec<&Vec<u64>> = attributes
        .into_iter()
        .filter(|r| !r.is_empty() && r.iter().all(|x| x & ATTRIBUTE != 0))
        .collect();
    // An adjective in no case the preposition governs: it isn't governing
    // this phrase («что за головная боль» — the nominative).
    if let Some(g) = governed {
        if attributes
            .iter()
            .any(|a| !a.iter().any(|&x| cases(x) & g != 0))
        {
            governed = None;
        }
    }
    if governed.is_none() && attributes.is_empty() {
        return false;
    }
    let nominal: Vec<u64> = word
        .iter()
        .copied()
        .filter(|r| r & NOMINAL != 0 && r & CASE != 0)
        .collect();
    let fits = |r: u64| {
        governed.is_none_or(|g| cases(r) & g != 0)
            && attributes
                .iter()
                .all(|a| a.iter().any(|&x| x & ATTRIBUTE != 0 && agree(x, r)))
    };
    nominal.len() == word.len() && !nominal.iter().any(|&r| fits(r))
}

/// A governing word's frame, learned from text (tools/build_frames.py): how
/// much likelier than anywhere the next word is something other than a noun
/// phrase, or one in each case (log ratios, nats) — «к» the dative, «с» the
/// instrumental, «помочь» the dative, «видел» the accusative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub other: f32,
    /// Nominative, genitive, dative, accusative, instrumental, locative.
    pub cases: [f32; 6],
}

const FRAME_CASES: [u64; 6] = [NOMN, GENT, DATV, ACCS, ABLT, LOCT];

impl Frame {
    /// The log ratio of a reading's case (the second genitive… as the first).
    fn case(&self, r: u64) -> Option<f32> {
        let c = cases(r);
        FRAME_CASES
            .iter()
            .zip(self.cases)
            .filter(|(k, _)| c & **k != 0)
            .map(|(_, v)| v)
            .reduce(f32::max)
    }

    /// How much likelier `word` (its readings) comes right after the
    /// governor than anywhere: by its best reading — a noun phrase by its
    /// case, anything else as `other` («казни» is a noun and a verb).
    pub fn next(&self, word: &[u64]) -> f32 {
        let nominal = word.iter().filter(|&&r| r & NOMINAL != 0 && r & CASE != 0);
        let case = nominal.filter_map(|&r| self.case(r)).reduce(f32::max);
        let other = word
            .iter()
            .any(|&r| r & NOMINAL == 0 || r & CASE == 0)
            .then_some(self.other);
        case.into_iter()
            .chain(other)
            .reduce(f32::max)
            .unwrap_or(0.0)
    }
}

/// After adjectives, learned the same way: how much likelier than anywhere
/// the next word is something else, a noun phrase that agrees, one that
/// doesn't (log ratios, nats).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttrWeights {
    pub other: f32,
    pub agree: f32,
    pub disagree: f32,
    /// `ln P(other)`: how often anything but a noun phrase comes anywhere.
    pub base_other: f32,
}

/// The phrase before the next word, read back from its end (as [`misfit`]
/// reads it): the word governing it, the adjectives between, or a subject
/// pronoun right before.
#[derive(Debug, Default)]
pub struct Walk<'a> {
    /// The governing word's index in the phrase: a preposition, a verb, a
    /// noun — the first word back that isn't an adjective, «и»/«или» between
    /// them, or (with no adjective yet) an adverb or particle.
    pub governor: Option<usize>,
    /// The adjectives the next word agrees with (words that can't be
    /// anything else).
    pub attributes: Vec<&'a Vec<u64>>,
    pub subject: Option<u64>,
}

pub fn walk<'a>(phrase: &'a [(&str, Vec<u64>)]) -> Walk<'a> {
    let attributive = |r: &Vec<u64>| r.iter().any(|x| x & ATTRIBUTE != 0 && x & CASE != 0);
    let strict = |r: &Vec<u64>| !r.is_empty() && r.iter().all(|x| x & ATTRIBUTE != 0);
    let mut out = Walk::default();
    let mut collected = 0;
    for (i, (w, readings)) in phrase.iter().enumerate().rev().take(5) {
        if w.starts_with("котор") {
            return out;
        }
        if collected == phrase.len() - 1 - i && attributive(readings) {
            // Adjectives of one phrase agree with each other; one that may
            // be an adjective («этому») by its adjective readings.
            let next = out.attributes.last();
            let as_attribute = readings.iter().filter(|&&x| x & ATTRIBUTE != 0);
            if next.is_some_and(|n| {
                !as_attribute
                    .clone()
                    .any(|&x| n.iter().any(|&y| agree(x, y)))
            }) {
                return out;
            }
            if strict(readings) {
                out.attributes.push(readings);
            }
            collected += 1;
            continue;
        }
        if collected > 0 && matches!(*w, "и" | "или") {
            collected += 1;
            continue;
        }
        if collected == 0 {
            if let Some(s) = subject(w) {
                out.subject = Some(s);
                return out;
            }
            if !readings.is_empty() && readings.iter().all(|r| r & (ADVB | PRCL) != 0) {
                collected += 1;
                continue;
            }
        }
        out.governor = Some(i);
        return out;
    }
    out
}

/// Whether a word governs the noun phrase after it across adjectives — a
/// preposition, a verb (or its infinitive, participle, gerund) — rather than
/// standing in a phrase of its own («мне свой номер»: the verb before
/// governs; «Мария высокого мнения»: not the noun).
pub fn governing(readings: &[u64]) -> bool {
    readings
        .iter()
        .any(|r| r & (PREP | VERB | INFN | GRND | PRTF | PRTS) != 0)
}

/// How the phrase before weighs `word` (its readings), in nats: agreement
/// with the adjectives before it; `frame`, the frame of the word governing
/// them — given only when words stand between the two (right after its
/// governor, the context model weighs a word itself), and then only for a
/// noun phrase (its `other` is what comes right after the governor: «в тот
/// же день»); a verb against its subject pronoun.
pub fn phrase_score(walk: &Walk, frame: Option<&Frame>, attr: &AttrWeights, word: &[u64]) -> f32 {
    if word.is_empty() {
        return 0.0;
    }
    if let Some(s) = walk.subject {
        let verbs = word.iter().filter(|&&r| r & VERB != 0).count();
        let fits = word.iter().any(|&r| r & VERB != 0 && fits_subject(r, s));
        return if verbs == word.len() && !fits {
            -1.0
        } else {
            0.0
        };
    }
    let nominal: Vec<u64> = word
        .iter()
        .copied()
        .filter(|&r| r & NOMINAL != 0 && r & CASE != 0)
        .collect();
    let agreeing: Vec<u64> = nominal
        .iter()
        .copied()
        .filter(|&r| {
            walk.attributes
                .iter()
                .all(|a| a.iter().any(|&x| agree(x, r)))
        })
        .collect();
    let mut score = 0.0;
    if !walk.attributes.is_empty() && nominal.len() == word.len() && agreeing.is_empty() {
        score += attr.disagree;
    }
    // The frame counts when the adjectives can take a case it governs
    // («что за головная боль»: not the accusative of «за»).
    let frame = frame.filter(|f| {
        walk.attributes
            .iter()
            .all(|a| a.iter().any(|&x| f.case(x).is_some_and(|v| v > -1.5)))
    });
    // Across adjectives a noun phrase is on its way: the frame's cases
    // among noun phrases alone («можешь весь день»: after «можешь» mostly an
    // infinitive, but a noun phrase there may well be in the accusative).
    // Only across words that can't be anything but adjectives: «могу её
    // найти», «о том же».
    let frame = frame.filter(|_| !walk.attributes.is_empty());
    if let Some(f) = frame.filter(|_| !nominal.is_empty()) {
        let base = attr.base_other.exp().min(0.99);
        let here = (base * f.other.exp()).min(0.99);
        let nominal_share = ((1.0 - here) / (1.0 - base)).ln();
        let readings = if agreeing.is_empty() {
            &nominal
        } else {
            &agreeing
        };
        score += f.next(readings) - nominal_share;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(tags: &[&str]) -> Vec<u64> {
        tags.iter().map(|t| parse(t)).collect()
    }

    #[test]
    fn a_preposition_governs_the_phrase_after_it() {
        let v = ("в", r(&["PREP"]));
        let big = (
            "большом",
            r(&["ADJF,masc,sing,loct", "ADJF,neut,sing,loct"]),
        );
        let dome = r(&["NOUN,inan,masc,sing,loct"]);
        let dom = r(&["NOUN,inan,masc,sing,nomn", "NOUN,inan,masc,sing,accs"]);
        let doma = r(&[
            "NOUN,inan,masc,sing,gent",
            "NOUN,inan,masc,plur,nomn",
            "NOUN,inan,masc,plur,accs",
        ]);
        // в большом доме, not в большом дом / дома.
        assert!(!misfit(&[v.clone(), big.clone()], &dome));
        assert!(misfit(&[v.clone(), big.clone()], &dom));
        assert!(misfit(&[v.clone(), big.clone()], &doma));
        // в дом (accusative) is fine right after the preposition.
        assert!(!misfit(std::slice::from_ref(&v), &dom));
        // с другом, not с друг.
        let s = ("с", r(&["PREP"]));
        assert!(!misfit(
            std::slice::from_ref(&s),
            &r(&["NOUN,anim,masc,sing,ablt"])
        ));
        assert!(misfit(&[s], &r(&["NOUN,anim,masc,sing,nomn"])));
        // An adverb or a verb has nothing to agree about.
        assert!(!misfit(&[v], &r(&["ADVB"])));
    }

    #[test]
    fn attributes_agree_through_and() {
        let phrase = [
            ("в", r(&["PREP"])),
            ("большом", r(&["ADJF,masc,sing,loct"])),
            ("и", r(&["CONJ"])),
            ("светлом", r(&["ADJF,masc,sing,loct"])),
        ];
        assert!(!misfit(&phrase, &r(&["NOUN,inan,masc,sing,loct"])));
        assert!(misfit(&phrase, &r(&["NOUN,inan,femn,sing,loct"])));
        // Without a preposition the attributes still agree: неведомых дорожках.
        let phrase = [(
            "неведомых",
            r(&["ADJF,plur,gent", "ADJF,plur,loct", "ADJF,plur,accs"]),
        )];
        assert!(!misfit(&phrase, &r(&["NOUN,inan,femn,plur,loct"])));
        assert!(misfit(&phrase, &r(&["NOUN,inan,femn,sing,nomn"])));
    }

    #[test]
    fn a_verb_agrees_with_its_pronoun() {
        // A verb that agrees gains nothing (0), one that doesn't loses (-1).
        let she = [("она", r(&["NPRO,femn,3per,sing,nomn"]))];
        let said_f = r(&["VERB,perf,tran,femn,sing,past,indc"]);
        let said_m = r(&["VERB,perf,tran,masc,sing,past,indc"]);
        assert!(!misfit(&she, &said_f));
        assert!(misfit(&she, &said_m));
        // «не» and adverbs between: она быстро не пошла.
        let phrase = [("она", r(&["NPRO"])), ("не", r(&["PRCL"]))];
        assert!(!misfit(&phrase, &said_f));
        let we = [("мы", r(&["NPRO"]))];
        assert!(!misfit(&we, &r(&["VERB,impf,1per,plur,pres,indc"])));
        assert!(misfit(&we, &r(&["VERB,impf,3per,sing,pres,indc"])));
        // «я иди» isn't, «ты иди» is.
        let go = r(&["VERB,sing"]);
        assert!(misfit(&[("я", r(&["NPRO"]))], &go));
        assert!(!misfit(&[("ты", r(&["NPRO"]))], &go));
        // я — past tense of either gender.
        assert!(!misfit(&[("я", r(&["NPRO"]))], &said_m));
        assert!(!misfit(&[("я", r(&["NPRO"]))], &said_f));
    }

    #[test]
    fn a_pronoun_that_may_be_an_attribute_asks_for_nothing() {
        // «это моя книга»: «это» is mostly a pronoun; «в это время» still
        // reaches its preposition through it.
        let eto = (
            "это",
            r(&[
                "NPRO,neut,sing,nomn",
                "PRCL",
                "ADJF,neut,sing,nomn",
                "ADJF,neut,sing,accs",
            ]),
        );
        assert!(!misfit(
            std::slice::from_ref(&eto),
            &r(&["ADJF,femn,sing,nomn"])
        ));
        let v = ("в", r(&["PREP"]));
        assert!(!misfit(
            &[v.clone(), eto.clone()],
            &r(&["NOUN,neut,sing,accs"])
        ));
        assert!(misfit(&[v, eto], &r(&["NOUN,neut,sing,datv"])));
    }

    #[test]
    fn a_word_that_may_be_something_else_is_not_ruled_out() {
        // «один из»: «из» is also an abbreviated noun, in any case.
        let one = [("один", r(&["ADJF,masc,sing,nomn"]))];
        assert!(!misfit(&one, &r(&["PREP", "NOUN,masc,sing,gent"])));
        assert!(misfit(&one, &r(&["NOUN,masc,sing,gent"])));
        // «он мыла»: no, «мыла» the noun — yes.
        let he = [("он", r(&["NPRO"]))];
        assert!(!misfit(
            &he,
            &r(&["VERB,femn,sing,past", "NOUN,neut,sing,gent"])
        ));
        assert!(misfit(&he, &r(&["VERB,femn,sing,past"])));
    }

    #[test]
    fn not_every_preposition_governs_what_follows() {
        // «что за головная боль»: the nominative.
        let phrase = [
            ("что", r(&["CONJ", "NPRO,neut,sing,nomn"])),
            ("за", r(&["PREP"])),
            ("головная", r(&["ADJF,femn,sing,nomn"])),
        ];
        assert!(!misfit(
            &phrase,
            &r(&["NOUN,femn,sing,nomn", "NOUN,femn,sing,accs"])
        ));
        assert!(misfit(&phrase, &r(&["NOUN,masc,sing,nomn"])));
        // «которую ты»: a clause starts.
        let which = [("которую", r(&["ADJF,femn,sing,accs"]))];
        assert!(!misfit(&which, &r(&["NPRO,sing,nomn"])));
    }

    #[test]
    fn adjectives_that_disagree_are_two_phrases() {
        // «к этому никакого отношения»: «этому» goes with «к», «никакого»
        // with «отношения».
        let phrase = [
            ("к", r(&["PREP"])),
            ("этому", r(&["ADJF,masc,sing,datv", "ADJF,neut,sing,datv"])),
            (
                "никакого",
                r(&["ADJF,masc,sing,gent", "ADJF,neut,sing,gent"]),
            ),
        ];
        assert!(!misfit(
            &phrase,
            &r(&["NOUN,neut,sing,gent", "NOUN,neut,plur,nomn"])
        ));
        assert!(misfit(&phrase, &r(&["NOUN,femn,sing,gent"])));
    }

    fn learned() -> (Frame, AttrWeights) {
        // «из»: the genitive, hardly anything but a noun phrase.
        let iz = Frame {
            other: -1.9,
            cases: [-1.0, 1.6, -0.4, 0.4, -1.1, 0.6],
        };
        let attr = AttrWeights {
            other: -1.1,
            agree: 0.4,
            disagree: -2.7,
            base_other: 0.5f32.ln(),
        };
        (iz, attr)
    }

    #[test]
    fn a_learned_frame_weighs_the_word_across_adjectives() {
        let (iz, attr) = learned();
        let phrase = [
            ("из", r(&["PREP"])),
            (
                "старого",
                r(&[
                    "ADJF,masc,sing,gent",
                    "ADJF,neut,sing,gent",
                    "ADJF,masc,sing,accs",
                ]),
            ),
        ];
        let w = walk(&phrase);
        assert_eq!(w.governor, Some(0));
        assert_eq!(w.attributes.len(), 1);
        let doma = r(&[
            "NOUN,masc,sing,gent",
            "NOUN,masc,plur,nomn",
            "NOUN,masc,plur,accs",
        ]);
        let dom = r(&["NOUN,masc,sing,nomn", "NOUN,masc,sing,accs"]);
        let dome = r(&["NOUN,masc,sing,loct"]);
        let s = |word: &[u64]| phrase_score(&w, Some(&iz), &attr, word);
        // из старого дома: the genitive; дом agrees (accusative) but «из»
        // hardly takes it; доме doesn't agree at all.
        assert!(s(&doma) > s(&dom), "{} {}", s(&doma), s(&dom));
        assert!(s(&dom) > s(&dome));
        assert!(s(&dome) < -2.0);
        // Right after its governor the frame is the context model's (next);
        // across adjectives a particle isn't weighed («из того же дома»).
        assert!(iz.next(&doma) > iz.next(&r(&["PREP"])));
        assert_eq!(s(&r(&["PRCL"])), 0.0);
        assert!(governing(&r(&["PREP"])) && !governing(&r(&["NPRO,sing,datv"])));
        // A noun that is a verb too: by its best reading.
        assert_eq!(iz.next(&r(&["NOUN,femn,sing,gent", "VERB,sing"])), 1.6);
    }

    #[test]
    fn the_walk_finds_the_governor_past_adverbs() {
        let phrase = [("помочь", r(&["INFN"])), ("очень", r(&["ADVB"]))];
        assert_eq!(walk(&phrase).governor, Some(0));
        let phrase = [("она", r(&["NPRO,femn,sing,nomn"]))];
        let w = walk(&phrase);
        assert!(w.subject.is_some() && w.governor.is_none());
        let (_, attr) = learned();
        assert_eq!(
            phrase_score(&w, None, &attr, &r(&["VERB,masc,sing,past"])),
            -1.0
        );
        assert_eq!(
            phrase_score(&w, None, &attr, &r(&["VERB,femn,sing,past"])),
            0.0
        );
    }

    #[test]
    fn a_noun_between_ends_the_phrase() {
        // в доме отца: «отца» isn't governed by «в».
        let phrase = [
            ("в", r(&["PREP"])),
            ("доме", r(&["NOUN,inan,masc,sing,loct"])),
        ];
        assert!(!misfit(&phrase, &r(&["NOUN,anim,masc,sing,gent"])));
    }
}
