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
    /// Animate, inanimate: in the accusative an adjective takes the
    /// genitive's form with an animate noun («такого друга»), the
    /// nominative's with an inanimate one («такой исход»).
    pub const ANIM: u64 = 1 << 48;
    pub const INAN: u64 = 1 << 49;

    pub const GENDER: u64 = MASC | FEMN | NEUT | MSF;
    pub const NUMBER: u64 = SING | PLUR;
    pub const CASE: u64 = NOMN | GENT | DATV | ACCS | ABLT | LOCT | VOCT | GEN2 | ACC2 | LOC2;
    pub const PERSON: u64 = PER1 | PER2 | PER3;
    /// Words of a noun phrase: they take its case (and agree).
    pub const NOMINAL: u64 = NOUN | ADJF | PRTF | NPRO | NUMR;
    /// Words that agree with the noun they go with.
    pub const ATTRIBUTE: u64 = ADJF | PRTF;
    /// Words that agree with their subject: a verb, a short adjective or
    /// participle («она рада», «они ранены»).
    pub const PREDICATE: u64 = VERB | ADJS | PRTS;
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
            "anim" => ANIM,
            "inan" => INAN,
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

/// The case bits with the second genitive, accusative and locative (чаю, в
/// лесу) counted as the first.
pub(crate) fn cases(r: u64) -> u64 {
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
        // «кто пришёл», «никто не знает», «ничто не изменилось».
        "кто" | "никто" => PER3 | SING,
        "ничто" => PER3 | SING | NEUT,
        _ => return None,
    })
}

/// A noun that can only be in the nominative («девочка», «мальчики»; not
/// «дом», which may be the accusative too) as a subject: third person, its
/// number and gender.
fn subject_noun(readings: &[u64]) -> Option<u64> {
    let only_nominative =
        !readings.is_empty() && readings.iter().all(|&r| r & NOUN != 0 && cases(r) == NOMN);
    if !only_nominative {
        return None;
    }
    let number = readings.iter().fold(0, |a, &r| a | (r & NUMBER));
    if number.count_ones() != 1 {
        return None;
    }
    let gender = readings.iter().fold(0, |a, &r| a | (r & GENDER));
    let gender = if gender.count_ones() == 1 { gender } else { 0 };
    Some(PER3 | number | gender)
}

/// How sure the graph must be that a word waits for its head still to come
/// for a noun that may be in the nominative to be taken for a subject.
const WAITING_SUBJECT: f32 = 0.5;

/// A noun that may be in the nominative («мать», «вода» — the genitive of a
/// rare «вод» too) waiting for a word still to come, as the sentence's graph
/// has it (`waits`, asked only for such a noun): a subject for its
/// predicate — by its nominative readings.
fn waiting_subject(readings: &[u64], waits: impl FnOnce() -> f32) -> Option<(u64, f32)> {
    let nominative: Vec<u64> = readings
        .iter()
        .copied()
        .filter(|&r| r & NOUN != 0 && cases(r) == NOMN)
        .collect();
    let s = subject_noun(&nominative)?;
    let p = waits();
    (p >= WAITING_SUBJECT).then_some((s, p))
}

/// A subject and how sure: a personal pronoun or a noun only in the
/// nominative (1), or one that may be, waiting for its head (`waits`, the
/// graph's chance).
fn subject_of(word: &str, readings: &[u64], waits: impl FnOnce() -> f32) -> Option<(u64, f32)> {
    subject(word)
        .or_else(|| subject_noun(readings))
        .map(|s| (s, 1.0))
        .or_else(|| waiting_subject(readings, waits))
}

/// The subject standing at `i` in the phrase: a personal pronoun or a noun
/// only in the nominative — and with another joined to it by «и»/«или»
/// («Эстелла и я»), the two together: plural, of the lesser person («мы»).
/// None when that other one can't be told (a name the dictionary doesn't
/// know, or the phrase starting at «и»).
fn subject_at<S: AsRef<str>>(
    phrase: &[(S, Vec<u64>)],
    waits: &mut dyn FnMut(usize) -> f32,
    i: usize,
) -> Option<(u64, f32)> {
    let (w, r) = &phrase[i];
    let (s, sure) = subject_of(w.as_ref(), r, || waits(i))?;
    let joined = i >= 1 && matches!(phrase[i - 1].0.as_ref(), "и" | "или");
    if !joined {
        return Some((s, sure));
    }
    let (w, r) = phrase.get(i.checked_sub(2)?)?;
    let nominative = r
        .iter()
        .any(|&x| x & (NOUN | NPRO) != 0 && cases(x) & NOMN != 0);
    let other = subject(w.as_ref()).or_else(|| nominative.then_some(PER3))?;
    let persons = (s | other) & PERSON;
    let person = [PER1, PER2, PER3]
        .into_iter()
        .find(|&p| persons & p != 0)
        .unwrap_or(PER3);
    Some((person | PLUR, sure))
}

/// Two readings agree (an attribute and what it goes with): a case, the
/// number and, in the singular, the gender in common — in the accusative,
/// the animacy too («такого исход» doesn't: «такого» is accusative only with
/// an animate noun).
fn agree(a: u64, b: u64) -> bool {
    if cases(a) & cases(b) == 0 || a & b & NUMBER == 0 {
        return false;
    }
    let animacy = |r: u64| r & (ANIM | INAN);
    if cases(a) & cases(b) == ACCS && animacy(a) != 0 && animacy(b) != 0 && animacy(a) != animacy(b)
    {
        return false;
    }
    let singular = a & b & SING != 0;
    let (ga, gb) = (a & GENDER, b & GENDER);
    !singular || ga == 0 || gb == 0 || ga & gb != 0 || (ga | gb) & MSF != 0
}

/// A predicate reading fits its subject: the past tense, a short adjective
/// or participle by number (and gender in the singular of the third
/// person), the present and future by person and number, the imperative (no
/// tense) only «ты» and «вы».
fn fits_subject(pred: u64, subj: u64) -> bool {
    if pred & subj & NUMBER == 0 {
        return false;
    }
    let g = subj & GENDER;
    let gender = pred & SING == 0 || g == 0 || g & MSF != 0 || pred & g != 0;
    if pred & (ADJS | PRTS) != 0 {
        return gender;
    }
    if pred & (PAST | PRES | FUTR) == 0 {
        return subj & PER2 != 0;
    }
    if pred & PAST != 0 {
        return gender;
    }
    pred & PERSON == 0 || pred & subj & PERSON != 0
}

/// Whether a word (its readings) fits its subject: None when it may be
/// anything but a predicate («мыла» is a noun too).
fn fits(word: &[u64], subj: u64) -> Option<bool> {
    let predicate = !word.is_empty() && word.iter().all(|r| r & PREDICATE != 0);
    predicate.then(|| word.iter().any(|&r| fits_subject(r, subj)))
}

/// A word between a subject and its verb: an adverb or a particle — maybe a
/// short adjective too («быстро»), nothing else.
fn adverbial(readings: &[u64]) -> bool {
    !readings.is_empty()
        && readings.iter().any(|r| r & (ADVB | PRCL) != 0)
        && readings
            .iter()
            .all(|r| r & (ADVB | PRCL | ADJS | COMP | PRED) != 0)
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
pub fn misfit<S: AsRef<str>>(phrase: &[(S, Vec<u64>)], word: &[u64]) -> bool {
    if word.is_empty() {
        return false;
    }
    // Right after an active participle the word may be its object («сдавшие
    // экзамен», «с читающим книгу мальчиком»): nothing asked of it.
    if phrase
        .last()
        .is_some_and(|(w, r)| active_participle(w.as_ref(), r))
    {
        return false;
    }
    let attributive = |r: &Vec<u64>| r.iter().any(|x| x & ATTRIBUTE != 0 && x & CASE != 0);
    // Back from the word: attributes (and «и» between them) up to a
    // preposition; or a subject pronoun, maybe with adverbs and «не» between.
    let mut governed: Option<u64> = None;
    let mut attributes: Vec<&Vec<u64>> = Vec::new();
    let mut subj: Option<u64> = None;
    for (i, (w, readings)) in phrase.iter().rev().take(5).enumerate() {
        let w = w.as_ref();
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
        if !attributes.is_empty() && matches!(w, "и" | "или") {
            attributes.push(readings);
            continue;
        }
        if attributes.is_empty() {
            if subject(w).or_else(|| subject_noun(readings)).is_some() {
                subj = subject_at(phrase, &mut |_| 0.0, phrase.len() - 1 - i).map(|s| s.0);
                break;
            }
            if adverbial(readings) {
                continue;
            }
        }
        break;
    }
    // A predicate and its subject.
    if let Some(s) = subj {
        return fits(word, s) == Some(false);
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
    /// An infinitive («могут выдержать», «хочу понравиться»).
    pub infn: f32,
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
        let infn = word.iter().any(|&r| r & INFN != 0).then_some(self.infn);
        let other = word
            .iter()
            .any(|&r| r & INFN == 0 && (r & NOMINAL == 0 || r & CASE == 0))
            .then_some(self.other);
        case.into_iter()
            .chain(infn)
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
    /// `ln P(other)`: how often anything but a noun phrase (or an
    /// infinitive) comes anywhere…
    pub base_other: f32,
    /// …and `ln P(infinitive)`.
    pub base_infn: f32,
    /// After a subject, a predicate that fits it and one that doesn't: how
    /// much likelier than a predicate drawn from anywhere would be.
    pub subject_fits: f32,
    pub subject_misfits: f32,
    /// After a subject, how much likelier than anywhere each kind of word
    /// ([`kind`]): a predicate, hardly a gerund or an infinitive.
    pub after_subject: Option<[f32; 8]>,
    /// …and among the kinds a word's own frame lumps together (a
    /// predicate, a gerund, an adverb, a preposition or conjunction,
    /// anything else; 0 for the rest): right after the subject its frame
    /// knows the cases and the infinitive.
    pub after_subject_other: Option<[f32; 8]>,
    /// After «X и» (X in one case): the next noun phrase in X's case, or
    /// not («кошку и собаку»).
    pub coord: Option<(f32, f32)>,
    /// After a genitive and «не» with no subject («её не существовало»,
    /// «денег не было»): a predicate that is impersonal (neuter, third
    /// person singular), or not.
    pub impersonal: Option<(f32, f32)>,
}

/// What kind of word a reading is, as what follows a subject is counted
/// (tools/build_frames.py): a predicate, a gerund, an infinitive, an adverb
/// or particle, a noun phrase in the nominative, in another case, a
/// preposition or conjunction, anything else.
fn kind(r: u64) -> usize {
    if r & PREDICATE != 0 {
        0
    } else if r & GRND != 0 {
        1
    } else if r & INFN != 0 {
        2
    } else if r & (ADVB | PRCL | PRED | COMP) != 0 {
        3
    } else if r & NOMINAL != 0 && r & CASE != 0 {
        if cases(r) & NOMN != 0 {
            4
        } else {
            5
        }
    } else if r & (PREP | CONJ) != 0 {
        6
    } else {
        7
    }
}

/// The phrase before the next word, read back from its end (as [`misfit`]
/// reads it): the word governing it, the adjectives between, or a subject
/// pronoun right before.
#[derive(Clone, Debug, Default)]
pub struct Walk {
    /// The governing word's index in the phrase: a preposition, a verb, a
    /// noun — the first word back that isn't an adjective, «и»/«или» between
    /// them, or (with no adjective yet) an adverb or particle.
    pub governor: Option<usize>,
    /// The adjectives the next word agrees with (words that can't be
    /// anything else).
    pub attributes: Vec<Vec<u64>>,
    pub subject: Option<u64>,
    /// How unsure the subject is (0: it can't be anything else; else 1 − the
    /// graph's chance that the noun waits for its predicate as the subject).
    pub subject_doubt: f32,
    /// The subject stands right before the word (no adverb between).
    pub subject_next: bool,
    /// No subject but a genitive before «не» («её не [существовало]»): the
    /// predicate impersonal.
    pub impersonal: bool,
    /// When the governor is a noun or pronoun: the verb whose clause its
    /// phrase stands in, back over noun phrases and adverbs («дал книгу»,
    /// «помог ему») — what else it takes comes next.
    pub clause: Option<usize>,
    /// Right after «X и»/«X или», X a noun phrase in one case: that case.
    pub coord: Option<u64>,
    /// The word index of the adjective nearest the word, and how much less
    /// likely than a noun phrase anywhere a noun phrase that doesn't agree
    /// comes after that very word (learned, when it has its own: «всем» is
    /// mostly on its own — «всем привет»); `AttrWeights::disagree` if not.
    pub nearest_attribute: Option<usize>,
    pub attribute_disagree: Option<f32>,
    /// The governor is «два», «три», «четыре», «оба», «полтора»: the
    /// adjectives after it needn't share the noun's number or case («два
    /// больших стола», «две большие книги»).
    pub paucal: bool,
}

/// The numerals after which a noun is in the genitive singular and its
/// adjectives in the plural.
fn paucal(word: &str) -> bool {
    matches!(
        word,
        "два" | "две" | "три" | "четыре" | "оба" | "обе" | "полтора" | "полторы"
    )
}

/// The verb whose clause the noun phrase ending at `g` stands in (see
/// [`Walk::clause`]); not past a preposition.
fn clause<S: AsRef<str>>(phrase: &[(S, Vec<u64>)], g: usize) -> Option<usize> {
    let r = &phrase[g].1;
    let noun = !r.is_empty() && r.iter().all(|&x| x & (NOUN | NPRO) != 0 && x & CASE != 0);
    if !noun {
        return None;
    }
    for k in (g.saturating_sub(4)..g).rev() {
        let rk = &phrase[k].1;
        if rk.is_empty() {
            return None;
        }
        if rk.iter().all(|&x| x & (VERB | INFN | GRND) != 0) {
            return Some(k);
        }
        let nominal = rk.iter().all(|&x| x & NOMINAL != 0 && x & CASE != 0);
        if !nominal && !adverbial(rk) {
            return None;
        }
    }
    None
}

/// An active participle («сдавший», «читающего», «учащийся»): it takes an
/// object as its verb does, so the word after it needn't agree with it. By
/// its form — the stem before the adjective's ending ends in ш or щ (-вш-,
/// -ш-, -ущ-, -ющ-, -ащ-, -ящ-); a passive one in н or т, or м.
pub fn active_participle(word: &str, readings: &[u64]) -> bool {
    if !readings.iter().any(|&r| r & PRTF != 0) {
        return false;
    }
    let w = word.strip_suffix("ся").or_else(|| word.strip_suffix("сь")).unwrap_or(word);
    const ENDINGS: [&str; 24] = [
        "ими", "ыми", "его", "ого", "ему", "ому", "ая", "яя", "ое", "ее", "ие", "ые", "ий", "ый",
        "ой", "ей", "ую", "юю", "им", "ым", "ем", "ом", "их", "ых",
    ];
    ENDINGS
        .iter()
        .find_map(|e| w.strip_suffix(e))
        .and_then(|stem| stem.chars().last())
        .is_some_and(|c| c == 'ш' || c == 'щ')
}

pub fn walk<S: AsRef<str>>(phrase: &[(S, Vec<u64>)]) -> Walk {
    walk_waiting(phrase, &mut |_| 0.0)
}

/// [`walk`], with the sentence's graph: `waits(i)`, the chance that the
/// phrase word `i` has its head still to come (asked only where it
/// decides) — a noun waiting may be the subject though it may be in
/// another case too.
pub fn walk_waiting<S: AsRef<str>>(
    phrase: &[(S, Vec<u64>)],
    waits: &mut dyn FnMut(usize) -> f32,
) -> Walk {
    let attributive = |r: &Vec<u64>| r.iter().any(|x| x & ATTRIBUTE != 0 && x & CASE != 0);
    let strict = |r: &Vec<u64>| !r.is_empty() && r.iter().all(|x| x & ATTRIBUTE != 0);
    let mut out = Walk::default();
    let mut collected = 0;
    // Words collected that are only adverbs or particles: a subject may
    // stand before them («девочка быстро побежала», «я не знаю»).
    let mut adverbs = 0;
    let mut negated = false;
    for (i, (w, readings)) in phrase.iter().enumerate().rev().take(5) {
        let w = w.as_ref();
        if w.starts_with("котор") {
            return out;
        }
        // Right after an active participle: maybe its object, not its noun.
        if i + 1 == phrase.len() && active_participle(w, readings) {
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
                if out.attributes.is_empty() {
                    out.nearest_attribute = Some(i);
                }
                out.attributes.push(readings.clone());
            }
            collected += 1;
            continue;
        }
        if collected > 0 && matches!(w, "и" | "или") {
            collected += 1;
            continue;
        }
        if collected == adverbs {
            if subject_of(w, readings, || waits(i)).is_some() {
                let s = subject_at(phrase, waits, i);
                out.subject = s.map(|s| s.0);
                out.subject_doubt = s.map_or(0.0, |s| 1.0 - s.1);
                out.subject_next = i + 1 == phrase.len();
                return out;
            }
            if adverbial(readings) {
                negated |= w == "не";
                collected += 1;
                adverbs += 1;
                continue;
            }
            // A genitive before «не», no subject before it: impersonal.
            let cased: Vec<u64> = readings.iter().copied().filter(|r| r & CASE != 0).collect();
            let genitive = !cased.is_empty()
                && cased
                    .iter()
                    .all(|&r| r & (NOUN | NPRO) != 0 && cases(r) & NOMN == 0)
                && cased.iter().any(|&r| cases(r) & GENT != 0);
            let subject_before = phrase[..i]
                .iter()
                .any(|(w, r)| subject(w.as_ref()).or_else(|| subject_noun(r)).is_some());
            if negated && adverbs > 0 && genitive && !subject_before {
                out.impersonal = true;
            }
        }
        out.governor = Some(i);
        out.paucal = paucal(w);
        out.clause = clause(phrase, i);
        if collected == 0 && i >= 1 && matches!(w, "и" | "или") {
            let x = &phrase[i - 1].1;
            let nominal = !x.is_empty() && x.iter().all(|&r| r & NOMINAL != 0 && r & CASE != 0);
            let case = x.iter().fold(0, |a, &r| a | cases(r));
            if nominal && case.count_ones() == 1 {
                out.coord = Some(case);
            }
        }
        return out;
    }
    out
}

/// The verb a preposition at `g` hangs on: right before it, maybe adverbs
/// between («бежала за», «бежала быстро за»).
pub fn verb_before<S: AsRef<str>>(phrase: &[(S, Vec<u64>)], g: usize) -> Option<usize> {
    for k in (g.saturating_sub(3)..g).rev() {
        let r = &phrase[k].1;
        if !r.is_empty() && r.iter().all(|&x| x & (VERB | INFN | GRND) != 0) {
            return Some(k);
        }
        if !adverbial(r) {
            return None;
        }
    }
    None
}

/// Whether a word governs the noun phrase after it across adjectives — a
/// preposition, a verb (or its infinitive, participle, gerund) — rather than
/// standing in a phrase of its own («мне свой номер»: the verb before
/// governs; «Мария высокого мнения»: not the noun).
pub fn governing(readings: &[u64]) -> bool {
    readings
        .iter()
        .any(|r| r & (PREP | VERB | INFN | GRND | PRTF | PRTS | NUMR) != 0)
}

/// How the phrase before weighs `word` (its readings), in nats: agreement
/// with the adjectives before it; `frame`, the frame of the word governing
/// them — given only when words stand between the two (right after its
/// governor, the context model weighs a word itself), and then only for a
/// noun phrase (its `other` is what comes right after the governor: «в тот
/// же день»); a verb against its subject pronoun.
pub fn phrase_score(
    walk: &Walk,
    frame: Option<&Frame>,
    clause: Option<&Frame>,
    attr: &AttrWeights,
    word: &[u64],
) -> f32 {
    if word.is_empty() {
        return 0.0;
    }
    if walk.impersonal {
        if let (Some((fit, misfit)), Some(ok)) = (attr.impersonal, fits(word, PER3 | SING | NEUT)) {
            return if ok { fit } else { misfit };
        }
    }
    if let Some(s) = walk.subject {
        // What kind of word it is, by its likeliest reading: right after
        // the subject only what its own frame can't tell apart.
        let kinds = if walk.subject_next {
            attr.after_subject_other
        } else {
            attr.after_subject
        };
        let by_kind = kinds.map_or(0.0, |k| {
            word.iter().map(|&r| k[kind(r)]).fold(f32::MIN, f32::max)
        });
        let given = by_kind
            + match fits(word, s) {
                Some(true) => attr.subject_fits,
                Some(false) => attr.subject_misfits,
                None => 0.0,
            };
        // A subject the graph isn't sure of: as likely as it is the subject,
        // what it says; else nothing.
        let doubt = walk.subject_doubt.clamp(0.0, 1.0);
        return if doubt > 0.0 {
            ((1.0 - doubt) * given.exp() + doubt).ln()
        } else {
            given
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
    if !walk.attributes.is_empty()
        && !walk.paucal
        && nominal.len() == word.len()
        && agreeing.is_empty()
    {
        score += walk.attribute_disagree.unwrap_or(attr.disagree);
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
        let nominal_share = nominal_share(f, attr);
        let readings = if agreeing.is_empty() {
            &nominal
        } else {
            &agreeing
        };
        score += f.next(readings) - nominal_share;
    }
    // The clause's verb, two or more words back: what else it takes.
    if let Some(c) = clause.filter(|_| walk.clause.is_some()) {
        score += c.next(word);
    }
    // After «X и»: the case of X.
    if let (Some(x), Some((same, other))) = (walk.coord, attr.coord) {
        if !nominal.is_empty() {
            score += if nominal.iter().any(|&r| cases(r) & x != 0) {
                same
            } else {
                other
            };
        }
    }
    score
}

/// How much likelier than anywhere a noun phrase comes after the governor
/// at all (log ratio): what its frame's cases are measured against.
fn nominal_share(f: &Frame, attr: &AttrWeights) -> f32 {
    let (b_other, b_infn) = (attr.base_other.exp(), attr.base_infn.exp());
    let base = (b_other + b_infn).min(0.99);
    let here = (b_other * f.other.exp() + b_infn * f.infn.exp()).min(0.99);
    ((1.0 - here) / (1.0 - base)).ln()
}

/// The most [`phrase_score`] gives any word after this phrase (nats): a
/// search may skip what can't win even with it.
pub fn phrase_score_max(
    walk: &Walk,
    frame: Option<&Frame>,
    clause: Option<&Frame>,
    attr: &AttrWeights,
) -> f32 {
    if walk.subject.is_some() {
        let kinds = [attr.after_subject, attr.after_subject_other]
            .iter()
            .flatten()
            .flat_map(|k| k.iter().copied())
            .fold(0.0, f32::max);
        return attr.subject_fits.max(0.0) + kinds;
    }
    let across = frame
        .filter(|_| !walk.attributes.is_empty())
        .map_or(0.0, |f| {
            let best = f.cases.iter().copied().fold(f32::MIN, f32::max);
            (best - nominal_share(f, attr)).max(0.0)
        });
    let verb = clause.filter(|_| walk.clause.is_some()).map_or(0.0, |c| {
        c.cases
            .iter()
            .copied()
            .chain([c.other, c.infn])
            .fold(0.0, f32::max)
    });
    let coord = walk
        .coord
        .and(attr.coord)
        .map_or(0.0, |(same, _)| same.max(0.0));
    across + verb + coord
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_participles_by_their_form() {
        for w in ["сдавшие", "читающего", "учащийся", "принесший", "идущими", "лежащая"] {
            assert!(active_participle(w, &[PRTF]), "{w}");
        }
        for w in ["написанное", "забытый", "читаемый", "построенного", "умный"] {
            assert!(!active_participle(w, &[PRTF]), "{w}");
        }
        assert!(!active_participle("сдавшие", &[ADJF]), "a participle's reading needed");
    }

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
            infn: -3.0,
        };
        let attr = AttrWeights {
            other: -1.1,
            agree: 0.4,
            disagree: -2.7,
            base_other: 0.4f32.ln(),
            base_infn: 0.1f32.ln(),
            subject_fits: 0.6,
            subject_misfits: -2.0,
            after_subject: None,
            after_subject_other: None,
            coord: None,
            impersonal: None,
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
        let s = |word: &[u64]| phrase_score(&w, Some(&iz), None, &attr, word);
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
    fn animacy_decides_the_accusative() {
        // «такого исход»: «такого» is accusative only with an animate noun.
        let takogo = [(
            "такого",
            r(&[
                "ADJF,anim,masc,sing,accs",
                "ADJF,masc,sing,gent",
                "ADJF,neut,sing,gent",
            ]),
        )];
        assert!(misfit(
            &takogo,
            &r(&["NOUN,inan,masc,sing,accs", "NOUN,inan,masc,sing,nomn"])
        ));
        assert!(!misfit(&takogo, &r(&["NOUN,inan,masc,sing,gent"])));
        assert!(!misfit(
            &takogo,
            &r(&["NOUN,anim,masc,sing,accs", "NOUN,anim,masc,sing,gent"])
        ));
    }

    #[test]
    fn a_noun_in_the_nominative_is_a_subject_too() {
        let girl = [("девочка", r(&["NOUN,anim,femn,sing,nomn"]))];
        assert!(misfit(&girl, &r(&["VERB,masc,sing,past"])));
        assert!(!misfit(&girl, &r(&["VERB,femn,sing,past"])));
        // «дом» may be the accusative: nothing said.
        let house = [(
            "дом",
            r(&["NOUN,inan,masc,sing,nomn", "NOUN,inan,masc,sing,accs"]),
        )];
        assert!(!misfit(&house, &r(&["VERB,femn,sing,past"])));
        // Past adverbs (one a short adjective too) and «не»: «девочка
        // быстро не побежал» — «побежал» a short adjective as well
        // («побежалый»), and that one doesn't agree either.
        let (_, attr) = learned();
        let phrase = [
            ("девочка", r(&["NOUN,anim,femn,sing,nomn"])),
            ("быстро", r(&["ADJS,neut,sing", "ADVB"])),
            ("не", r(&["PRCL"])),
        ];
        let pobezhal = r(&["ADJS,masc,sing", "VERB,masc,sing,past"]);
        let w = walk(&phrase);
        assert!(w.subject.is_some());
        assert_eq!(
            phrase_score(&w, None, None, &attr, &pobezhal),
            attr.subject_misfits
        );
        assert!(misfit(&phrase, &pobezhal));
        // Two joined: «Эстелла и я идём» — «мы»; a name the dictionary
        // doesn't know: nothing said.
        let we = [
            ("мама", r(&["NOUN,anim,femn,sing,nomn"])),
            ("и", r(&["CONJ"])),
            ("я", r(&["NPRO,sing,nomn"])),
        ];
        assert!(!misfit(&we, &r(&["VERB,plur,1per,pres"])));
        assert!(misfit(&we, &r(&["VERB,sing,1per,pres"])));
        assert!(misfit(&we, &r(&["VERB,femn,sing,past"])));
        let estella = [
            ("эстелла", Vec::new()),
            ("и", r(&["CONJ"])),
            ("я", r(&["NPRO,sing,nomn"])),
        ];
        assert!(walk(&estella).subject.is_none());
        assert!(!misfit(&estella, &r(&["VERB,plur,1per,pres"])));
        // A short adjective agrees: «девочка рада», not «рад».
        let girl = [("девочка", r(&["NOUN,anim,femn,sing,nomn"]))];
        assert!(misfit(&girl, &r(&["ADJS,masc,sing"])));
        assert!(!misfit(&girl, &r(&["ADJS,femn,sing"])));
        // Common gender: «сирота пришёл», «сирота пришла».
        let orphan = [("сирота", r(&["NOUN,anim,ms-f,sing,nomn"]))];
        assert!(!misfit(&orphan, &r(&["VERB,masc,sing,past"])));
        assert!(!misfit(&orphan, &r(&["VERB,femn,sing,past"])));
    }

    #[test]
    fn the_verb_of_the_clause_expects_what_else_it_takes() {
        // «дал книгу [другу]»: the governor is «книгу», the clause's verb
        // «дал» — after a noun phrase, the dative likelier.
        let (_, attr) = learned();
        let dal = Frame {
            other: -0.6,
            cases: [0.9, -0.6, 0.6, 0.8, -0.9, -0.4],
            infn: 0.5,
        };
        let phrase = [
            ("дал", r(&["VERB,masc,sing,past"])),
            ("книгу", r(&["NOUN,inan,femn,sing,accs"])),
        ];
        let w = walk(&phrase);
        assert_eq!((w.governor, w.clause), (Some(1), Some(0)));
        let s = |word: &[u64]| phrase_score(&w, None, Some(&dal), &attr, word);
        assert!(s(&r(&["NOUN,anim,masc,sing,datv"])) > s(&r(&["NOUN,anim,masc,sing,ablt"])));
        // Not past a preposition: «дал в руки» — the phrase isn't the verb's.
        let phrase = [
            ("дал", r(&["VERB,masc,sing,past"])),
            ("в", r(&["PREP"])),
            ("руки", r(&["NOUN,inan,femn,plur,accs"])),
        ];
        assert_eq!(walk(&phrase).clause, None);
    }

    #[test]
    fn after_x_and_the_case_of_x() {
        // «кошку и [собаку]»: the accusative again, not «собака».
        let (_, mut attr) = learned();
        attr.coord = Some((1.6, -1.9));
        let phrase = [
            ("кошку", r(&["NOUN,anim,femn,sing,accs"])),
            ("и", r(&["CONJ"])),
        ];
        let w = walk(&phrase);
        assert_eq!(w.coord, Some(ACCS));
        let s = |word: &[u64]| phrase_score(&w, None, None, &attr, word);
        assert!(s(&r(&["NOUN,anim,femn,sing,accs"])) > s(&r(&["NOUN,anim,femn,sing,nomn"])));
        // A verb after «и» is no noun phrase: nothing said.
        assert_eq!(s(&r(&["VERB,femn,sing,past"])), 0.0);
    }

    #[test]
    fn after_a_subject_a_predicate_not_a_gerund() {
        let (_, mut attr) = learned();
        attr.after_subject = Some([1.2, -0.7, -2.2, 0.5, -0.9, -1.1, -0.4, -0.7]);
        attr.after_subject_other = Some([0.45, -1.5, 0.0, -0.2, 0.0, 0.0, -1.1, -1.5]);
        let phrase = [("девочка", r(&["NOUN,anim,femn,sing,nomn"]))];
        let w = walk(&phrase);
        let s = |word: &[u64]| phrase_score(&w, None, None, &attr, word);
        assert!(s(&r(&["VERB,femn,sing,past"])) - s(&r(&["GRND"])) > 2.0);
    }

    #[test]
    fn after_two_the_adjective_goes_its_own_way() {
        // «два больших стола»: the adjective plural, the noun singular.
        let (_, attr) = learned();
        let phrase = [
            ("два", r(&["NUMR,masc,nomn", "NUMR,masc,accs"])),
            (
                "больших",
                r(&["ADJF,plur,gent", "ADJF,plur,loct", "ADJF,anim,plur,accs"]),
            ),
        ];
        let w = walk(&phrase);
        assert!(w.paucal);
        assert_eq!(
            phrase_score(&w, None, None, &attr, &r(&["NOUN,inan,masc,sing,gent"])),
            0.0
        );
    }

    #[test]
    fn no_subject_but_a_genitive_before_ne() {
        // «её не существовало»: impersonal, not «существовала»; «я её не
        // видел» has its subject.
        let (_, mut attr) = learned();
        attr.impersonal = Some((0.8, -2.5));
        let her = ("её", r(&["NPRO,femn,sing,gent", "NPRO,femn,sing,accs"]));
        let ne = ("не", r(&["PRCL"]));
        let w = walk(&[her.clone(), ne.clone()]);
        assert!(w.impersonal);
        let s = |w: &Walk, word: &[u64]| phrase_score(w, None, None, &attr, word);
        assert!(s(&w, &r(&["VERB,neut,sing,past"])) > s(&w, &r(&["VERB,femn,sing,past"])));
        let w = walk(&[("я", r(&["NPRO,sing,nomn"])), her, ne]);
        assert!(!w.impersonal);
    }

    #[test]
    fn a_frame_expects_an_infinitive() {
        // After «могут»: an infinitive, hardly a finite verb.
        let mogut = Frame {
            other: -1.0,
            cases: [-2.4, -2.9, -2.2, -2.5, -2.8, -3.0],
            infn: 3.0,
        };
        assert!(mogut.next(&r(&["INFN"])) > mogut.next(&r(&["VERB,plur,3per,futr"])));
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
            phrase_score(&w, None, None, &attr, &r(&["VERB,masc,sing,past"])),
            attr.subject_misfits
        );
        assert_eq!(
            phrase_score(&w, None, None, &attr, &r(&["VERB,femn,sing,past"])),
            attr.subject_fits
        );
        // Not only a predicate: nothing said.
        assert_eq!(
            phrase_score(
                &w,
                None,
                None,
                &attr,
                &r(&["VERB,masc,sing,past", "NOUN,inan,neut,sing,gent"])
            ),
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
