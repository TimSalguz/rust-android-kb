//! Punctuation from the sentence's graph, by the rules of Russian
//! (tools/graph_marks.py, which measures them): each link says where its
//! marks go — a subordinate clause, a «который» clause, a participle or
//! gerund phrase, an address, a parenthesis: commas at their bounds; «а»,
//! «но» and joined clauses: a comma before the joining word; things listed
//! without «и»: a comma between; «…, сообщил он»; a subject and a predicate
//! noun with no verb: a dash; a question word in the main clause: «?».
//!
//! The words' parts of speech are their readings in the dictionary, the one
//! each word's link calls for ([`tag`]; tools/graph_tags.py) — no tagger.

use crate::gram::bit::*;
use crate::parser::Sentence;

/// A mark the graph puts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Comma,
    Dash,
    Period,
    Question,
}

/// A mark before a word: its place (the word's index, 0-based), the mark,
/// how sure the graph is of the link that puts it, and the rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub before: usize,
    pub mark: Mark,
    pub chance: f32,
    pub rule: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pos {
    Noun,
    Propn,
    Pron,
    Adj,
    Num,
    Adv,
    Adp,
    Part,
    Intj,
    Cconj,
    Sconj,
    Verb,
    X,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    Fin,
    Inf,
    Part,
    Conv,
}

#[derive(Clone, Copy, Debug)]
struct Tag {
    pos: Pos,
    form: Option<Form>,
    nominative: bool,
    short: bool,
}

const JOINING: &[&str] = &["и", "или", "а", "но", "да", "либо", "ни", "однако", "зато", "иль"];
const SUBORDINATING: &[&str] = &[
    "что", "чтобы", "если", "когда", "потому", "так", "как", "хотя", "пока", "раз", "будто",
    "словно", "ибо", "ежели", "дабы", "поскольку", "чем", "лишь", "едва",
];
const PARENTHETICAL: &[&str] = &[
    "конечно", "наверное", "например", "кстати", "впрочем", "во-первых", "во-вторых",
    "в-третьих", "кажется", "пожалуй", "видимо", "разумеется", "по-моему", "по-видимому",
    "вероятно", "очевидно", "безусловно", "бесспорно", "несомненно", "право", "правда", "итак",
    "следовательно", "напротив", "наоборот", "словом", "значит", "стало", "может", "казалось",
    "говорят", "признаться", "по-твоему", "по-вашему", "по-нашему",
];
const APART_MORE: &[&str] = &[
    "ну", "да", "нет", "ах", "ох", "эх", "ой", "вот", "наконец", "главное", "действительно",
    "бывает", "ну-ка", "ведь", "впрочем",
];
const INTERJECTIONS: &[&str] = &[
    "ах", "ох", "эх", "ой", "ай", "ух", "увы", "о", "ого", "угу", "ага", "эй", "фу", "ура", "хм",
    "блин", "ба", "эге", "ну-ну",
];
/// How sure a known interjection starting the sentence is set apart when the
/// graph read it otherwise (the comma model has its say in the keyboard).
const INTERJECTION_FIRST: f32 = 0.85;
/// The links that set a word apart (an interjection, an aside, an address).
const APART: &[&str] = &["discourse", "parataxis", "vocative"];
const SAYING: &[&str] = &[
    "сказа", "говор", "сообщ", "заяв", "отмет", "подчеркн", "добав", "пояснил", "уточн",
    "рассказ", "признал", "написал", "спрос", "ответ", "отвеча", "продолж", "прибав", "повтор",
    "крикн", "закрич", "прошепт", "шепн", "проговор", "восклик", "думал", "подумал", "решил",
    "заметил", "возраз", "объясн", "указ", "призна", "констатир", "передает", "пишет",
    "считает", "полагает", "уверен",
];
const ADVERSATIVE: &[&str] = &["а", "но", "однако", "зато", "да", "же"];
const JOINING_CC: &[&str] = &["и", "или", "либо", "ни", "да"];
const PAREN_PHRASES: &[&str] = &[
    "в частности", "кроме того", "к тому же", "таким образом", "с одной стороны",
    "с другой стороны", "в первую очередь", "в свою очередь", "по крайней мере", "на самом деле",
    "по сути", "как правило", "как известно", "как оказалось", "как выяснилось", "как сообщается",
    "как говорится", "к сожалению", "к счастью", "к слову", "по-видимому", "без сомнения",
    "конечно же", "по его словам", "по ее словам", "по их словам", "по моему мнению",
    "по словам", "по данным", "по мнению", "по информации", "по сообщению", "по оценкам",
    "по прогнозам", "по подсчетам", "по сведениям", "по версии",
];
const QUESTION: &[&str] = &["ли", "разве", "неужели", "неужто", "что-ли"];
const ASKING: &[&str] = &[
    "кто", "что", "где", "куда", "откуда", "когда", "почему", "зачем", "отчего", "как", "сколько",
    "насколько", "какой", "какая", "какое", "какие", "каков", "какова", "чей", "чья", "чье", "чьи",
    "каким", "какую", "какого", "какому", "каком", "каких", "кого", "кому", "кем", "ком", "чего",
    "чему", "чем", "чего-нибудь",
];

/// A word's part of speech and form: of its readings, the one its link
/// (`rel`) calls for.
fn tag(word: &str, readings: &[u64], rel: &str) -> Tag {
    let has = |b: u64| readings.iter().any(|&r| r & b != 0);
    let mut t = Tag {
        pos: Pos::X,
        form: None,
        nominative: false,
        short: false,
    };
    let is = |set: &[&str]| set.contains(&word);
    if is(JOINING) && matches!(rel, "cc" | "conj" | "advmod" | "discourse" | "mark") && has(CONJ) {
        t.pos = Pos::Cconj;
        return t;
    }
    if is(SUBORDINATING) && rel == "mark" {
        t.pos = Pos::Sconj;
        return t;
    }
    let verb = |t: &mut Tag, f| {
        t.pos = Pos::Verb;
        t.form = Some(f);
    };
    if has(GRND) && matches!(rel, "advcl" | "conj" | "root" | "parataxis" | "acl") {
        verb(&mut t, Form::Conv);
        return t;
    }
    if has(PRTF | PRTS) && matches!(rel, "acl" | "amod" | "conj" | "root" | "advcl" | "xcomp") {
        verb(&mut t, Form::Part);
        let r = readings.iter().find(|&&r| r & (PRTF | PRTS) != 0).copied().unwrap_or(0);
        t.short = r & PRTS != 0;
        t.nominative = r & NOMN != 0;
        return t;
    }
    if has(INFN)
        && matches!(
            rel,
            "xcomp" | "csubj" | "advcl" | "acl" | "ccomp" | "root" | "conj" | "nsubj" | "obj"
        )
    {
        verb(&mut t, Form::Inf);
        return t;
    }
    if has(VERB)
        && matches!(
            rel,
            "root" | "conj" | "advcl" | "ccomp" | "acl" | "parataxis" | "csubj" | "xcomp" | "cop" | "aux"
        )
    {
        // (A copula or an auxiliary is a verb to the rules: «был» in «у нее был».)
        verb(&mut t, Form::Fin);
        return t;
    }
    // A pronoun's reading first: «нее» is «она», not the rare noun «нея».
    let order = [
        (NPRO, Pos::Pron),
        (NOUN, Pos::Noun),
        (ADJF, Pos::Adj),
        (ADJS, Pos::Adj),
        (NUMR, Pos::Num),
        (ADVB, Pos::Adv),
        (PREP, Pos::Adp),
        (PRCL, Pos::Part),
        (INTJ, Pos::Intj),
        (PRED, Pos::Adv),
        (COMP, Pos::Adv),
        (CONJ, Pos::Cconj),
    ];
    for (want, pos) in order {
        let Some(&first) = readings.iter().find(|&&r| r & want != 0) else {
            continue;
        };
        t.pos = pos;
        if want & (NOUN | NPRO | ADJF | NUMR) != 0 {
            // The nominative when the link wants it (a subject).
            let nominative = readings.iter().any(|&r| r & want != 0 && r & NOMN != 0);
            t.nominative = if rel == "nsubj" { nominative } else { first & NOMN != 0 };
        }
        t.short = want == ADJS;
        return t;
    }
    if has(VERB) {
        verb(&mut t, Form::Fin);
    } else if has(GRND) {
        verb(&mut t, Form::Conv);
    } else if readings.is_empty() {
        t.pos = Pos::Propn;
    }
    t
}

/// The sentence as the rules read it (1-based, as the graph numbers it).
struct Read<'a> {
    w: &'a [String],
    heads: Vec<usize>,
    rels: Vec<&'a str>,
    tags: Vec<Tag>,
    sure: Vec<f32>,
    kids: Vec<Vec<usize>>,
}

impl Read<'_> {
    fn n(&self) -> usize {
        self.w.len()
    }
    fn word(&self, i: usize) -> &str {
        &self.w[i - 1]
    }
    fn rel(&self, i: usize) -> &str {
        self.rels[i - 1]
    }
    fn tag(&self, i: usize) -> Tag {
        self.tags[i - 1]
    }
    fn span(&self, i: usize) -> (usize, usize) {
        let (mut lo, mut hi) = (i, i);
        let mut seen = vec![false; self.n() + 1];
        seen[i] = true;
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            lo = lo.min(j);
            hi = hi.max(j);
            for &k in &self.kids[j] {
                if !seen[k] {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        (lo, hi)
    }
    fn finite(&self, i: usize) -> bool {
        let t = self.tag(i);
        t.pos == Pos::Verb && t.form == Some(Form::Fin)
    }
    fn clause(&self, i: usize) -> bool {
        if self.finite(i) {
            return true;
        }
        if self.kids[i]
            .iter()
            .any(|&k| matches!(self.rel(k), "nsubj" | "cop" | "mark" | "aux" | "expl"))
        {
            return true;
        }
        let t = self.tag(i);
        matches!(t.pos, Pos::Adj | Pos::Verb) && t.short
    }
    fn has_kid(&self, i: usize, rel: &str) -> bool {
        self.kids[i].iter().any(|&k| self.rel(k) == rel)
    }
}

/// The marks the graph of a whole sentence puts: before its words, and at
/// its end. `words` lowercase; `readings` each word's in the dictionary;
/// `relations` the parser's relation names.
pub fn place(
    words: &[String],
    readings: &[Vec<u64>],
    graph: &Sentence,
    relations: &[String],
) -> (Vec<Placed>, Mark) {
    let n = words.len();
    if n == 0 || graph.heads.len() != n || graph.relations.len() != n {
        return (Vec::new(), Mark::Period);
    }
    let heads: Vec<usize> = graph
        .heads
        .iter()
        .map(|row| {
            (0..row.len())
                .max_by(|&a, &b| row[a].total_cmp(&row[b]))
                .unwrap_or(0)
                .min(n)
        })
        .collect();
    let rels: Vec<&str> = graph
        .relations
        .iter()
        .map(|&(k, _)| relations.get(k).map_or("dep", String::as_str))
        .collect();
    let sure: Vec<f32> = graph
        .heads
        .iter()
        .zip(&graph.relations)
        .map(|(row, &(_, p))| row.iter().copied().fold(0.0, f32::max) * p)
        .collect();
    let mut kids = vec![Vec::new(); n + 1];
    for (i, &h) in heads.iter().enumerate() {
        kids[h].push(i + 1);
    }
    // How sure the graph is that a word is set apart: its head, and any of
    // the links that set it apart (they share the chance between them).
    let apart: Vec<f32> = (0..n)
        .map(|i| {
            let head = graph.heads[i].iter().copied().fold(0.0, f32::max);
            let rel = match graph.relation_chances.get(i) {
                Some(p) => p
                    .iter()
                    .enumerate()
                    .filter(|&(k, _)| relations.get(k).is_some_and(|r| APART.contains(&r.as_str())))
                    .map(|(_, &x)| x)
                    .sum(),
                None => graph.relations[i].1,
            };
            head * rel
        })
        .collect();
    let interjection: Vec<bool> = (0..n)
        .map(|i| INTERJECTIONS.contains(&words[i].as_str()) || readings[i].iter().any(|&r| r & INTJ != 0))
        .collect();
    let mut chance_by: Vec<Option<f32>> = vec![None; n];
    let tags = (0..n).map(|i| tag(&words[i], &readings[i], rels[i])).collect();
    let s = Read {
        w: words,
        heads,
        rels,
        tags,
        sure,
        kids,
    };
    let mut put: Vec<Option<(Mark, &'static str, usize)>> = vec![None; n + 2];
    let mut set = |at: usize, why: &'static str, by: usize| {
        if at >= 2 && at <= n && put[at].is_none() {
            put[at] = Some((Mark::Comma, why, by));
        }
    };
    for i in 1..=n {
        let (b, word, t) = (s.rel(i), s.word(i), s.tag(i));
        let (lo, hi) = s.span(i);
        let head = s.heads[i - 1];
        let mut around = |why| {
            set(lo, why, i);
            set(hi + 1, why, i);
        };
        if i == 1
            && n > 1
            && INTERJECTIONS.contains(&word)
            && s.kids[i].is_empty()
            && !matches!(b, "case" | "det" | "amod" | "nummod" | "fixed" | "flat" | "compound")
        {
            // A known interjection first, whatever the parser made of it: its
            // other readings («блин» a noun, the subject of «опоздал») are
            // the grammar's, not the sense's.
            chance_by[i - 1] = Some(apart[i - 1].max(INTERJECTION_FIRST));
            set(2, "interjection first", i);
        } else if b == "advcl" && t.form == Some(Form::Conv) {
            around("gerund");
        } else if b == "acl" && t.form == Some(Form::Part) {
            if lo > head {
                around("participle after");
            }
        } else if b == "acl" && s.clause(i) {
            around("relative");
        } else if matches!(b, "advcl" | "ccomp") && s.clause(i) {
            around(if b == "advcl" { "advcl" } else { "ccomp" });
        } else if b == "vocative" {
            around("address");
        } else if (b == "discourse" && INTERJECTIONS.contains(&word))
            || (b == "parataxis" && (PARENTHETICAL.contains(&word) || APART_MORE.contains(&word)))
        {
            if b == "discourse" {
                chance_by[i - 1] = Some(apart[i - 1]);
            }
            around("parenthetical");
        } else if s.kids[i].is_empty()
            && matches!(b, "discourse" | "parataxis")
            && (interjection[i - 1] || lo == 1 && head > 1 && b == "parataxis")
        {
            // An interjection («О, а тут…», «Угу, …»), or a word alone at the
            // sentence's start set apart from what follows («Слушай, ты…»).
            chance_by[i - 1] = Some(apart[i - 1]);
            around(if interjection[i - 1] { "interjection" } else { "apart at the start" });
        } else if b == "parataxis" && lo > head && SAYING.iter().any(|p| word.starts_with(p)) {
            around("said");
        } else if b == "parataxis" || b == "appos" {
            // An aside, a quote's frame, an apposition: dash, colon,
            // brackets as often as commas — the writer's.
        } else if matches!(b, "obl" | "advcl") && s.kids[i].iter().any(|&k| s.word(k) == "несмотря")
        {
            around("несмотря на");
        } else if b == "conj" {
            let mut cc = s.kids[i]
                .iter()
                .copied()
                .find(|&k| k < i && s.rel(k) == "cc");
            // A joining word right before the conjunct, whatever it hangs on.
            if cc.is_none() && lo > 1 && s.tag(lo - 1).pos == Pos::Cconj {
                cc = Some(lo - 1);
            }
            let start = cc.unwrap_or(lo);
            match cc {
                Some(c) if ADVERSATIVE.contains(&s.word(c)) && !JOINING_CC.contains(&s.word(c)) => {
                    set(start, "а/но", i)
                }
                None => set(start, "listed", i),
                Some(c) => {
                    let joined_clauses = s.clause(i)
                        && s.clause(head)
                        && s.has_kid(i, "nsubj")
                        && head >= 1
                        && s.has_kid(head, "nsubj");
                    let repeated = s.kids.get(head).is_some_and(|ks| {
                        ks.iter()
                            .any(|&k| k < head && s.rel(k) == "cc" && s.word(k) == s.word(c))
                    });
                    if joined_clauses {
                        set(start, "clauses и", i);
                    } else if repeated {
                        set(start, "и…, и…", i);
                    }
                }
            }
        }
    }
    // Set parenthetical phrases, to the end of the phrase their last word
    // heads («по данным обсерватории»).
    for start in 1..=n {
        for phrase in PAREN_PHRASES {
            let parts: Vec<&str> = phrase.split(' ').collect();
            let k = parts.len();
            if start + k - 1 > n || (0..k).any(|j| s.word(start + j) != parts[j]) {
                continue;
            }
            let last = start + k - 1;
            let hi = if parts[0] == "по" && k == 2 {
                s.span(last).1.max(last)
            } else {
                last
            };
            set(start, "set phrase", last);
            set(hi + 1, "set phrase", last);
            break;
        }
    }
    let mut out: Vec<Placed> = put
        .iter()
        .enumerate()
        .filter_map(|(at, p)| {
            p.map(|(mark, rule, by)| Placed {
                before: at - 1,
                mark,
                chance: chance_by[by - 1].unwrap_or(s.sure[by - 1]),
                rule,
            })
        })
        .collect();
    out.extend(dashes(&s));
    out.sort_by_key(|p| p.before);
    (out, end_mark(&s))
}

/// Dashes: a subject and a predicate noun with no verb («Москва — столица»,
/// «Москва — это столица»), infinitives («жить — родине служить»).
fn dashes(s: &Read) -> Vec<Placed> {
    let mut out = Vec::new();
    for i in 1..=s.n() {
        if s.kids[i].iter().any(|&k| matches!(s.rel(k), "cop" | "aux")) {
            continue;
        }
        let Some(&k) = s.kids[i].iter().filter(|&&k| k < i && s.rel(k) == "nsubj").max() else {
            continue;
        };
        let (ti, tk) = (s.tag(i), s.tag(k));
        let nominal = matches!(ti.pos, Pos::Noun | Pos::Propn | Pos::Num)
            && ti.nominative
            && matches!(tk.pos, Pos::Noun | Pos::Propn)
            && tk.nominative;
        let infinitives = ti.form == Some(Form::Inf) && tk.form == Some(Form::Inf);
        if !(nominal || infinitives) {
            continue;
        }
        let lo = s.span(i).0;
        let eto = s.kids[i].iter().copied().filter(|&j| j < i && s.word(j) == "это").min();
        let at = eto.map_or(lo, |e| e.min(lo));
        if at > s.span(k).1 && at >= 2 {
            out.push(Placed {
                before: at - 1,
                mark: Mark::Dash,
                chance: s.sure[i - 1],
                rule: if nominal { "zero copula" } else { "infinitives" },
            });
        }
    }
    out
}

/// How the sentence ends: a question word or «ли» in its main clause — «?».
fn end_mark(s: &Read) -> Mark {
    let Some(root) = (1..=s.n()).find(|&i| s.heads[i - 1] == 0) else {
        return Mark::Period;
    };
    let asking = |j: usize| ASKING.contains(&s.word(j)) && !matches!(s.rel(j), "mark" | "cc" | "fixed");
    let clause: Vec<usize> = std::iter::once(root).chain(s.kids[root].iter().copied()).collect();
    for &j in &clause {
        if QUESTION.contains(&s.word(j)) || (j != root && asking(j)) {
            return Mark::Question;
        }
        if matches!(s.rel(j), "obl" | "obj" | "nsubj" | "advmod" | "nmod" | "iobj")
            && s.kids[j].iter().any(|&k| asking(k))
        {
            return Mark::Question;
        }
    }
    if asking(root) && root == 1 {
        Mark::Question
    } else {
        Mark::Period
    }
}
