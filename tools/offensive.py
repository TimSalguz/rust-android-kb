#!/usr/bin/env python3
"""Obscene words of a lexicon: the keyboard never suggests them or corrects
to them (a setting, on by default); typed letter by letter they stay as typed.

Usage: tools/offensive.py LANG LEXICON > OFFENSIVE.txt   (one word per line)

Russian: roots of мат, matched at the start of the word or of its lemma
(pymorphy3, when installed), after a prefix — so охуеть, заебись, выёбываться
are caught and страхую, тихую, похудеть, мудрый, бляха, себя are not. Other
languages: short hand-made lists of real profanity and slurs (a trailing `*`
takes every word starting so), not the broad "bad words" lists made for web
filters, which would hide comer, cerveja, martillo.
"""
import re
import sys

lang, lexicon = sys.argv[1], sys.argv[2]

# Prefixes the мат roots take (not a bare с/в/об: себе, вебинар, обед).
PREFIX = r"(?:на|по|за|от|отъ|вы|до|объ|раз|рас|разъ|при|про|пере|под|подъ|съ|у|въ|о|недо|ис|из|изъ|вз|взъ|ни)?"
RU = re.compile(
    r"^(?:" + PREFIX + r"(?:"
    r"ху(?:й|е|ё|я|и|ю)"                  # хуй, охуеть, нахуя (not похудеть, хулиган)
    r"|пизд|пезд"
    r"|[её]б(?:а|у|л|н|ё|и|ы|ь|с|к|е|о)"    # ебать, заебись, уёбок (not себя, небо)
    r")"
    r"|долбо[её]б"
    r"|(?:по|на|за)?бля(?:$|д|т)"          # бля, блядь, блять (not бляха)
    r"|муд(?:ак|ач|ил|оз|ищ)"              # мудак (not мудрый)
    r"|пид[оа]р|пидр(?:$|ил|ас)"           # (not спидран)
    r"|г[ао]ндон|залуп|шлюх"
    r"|сук(?:а|и|е|у|ой|ам|ами|ах)$|сучк(?:а|и|е|у|ой)$|сучар"  # (not сучок, сучковатый)
    r"|(?:по|на|ни)?хер(?:$|а$|у$|ом$|е$|ня|ов|ни)"  # хер, херня (not херувим, херес)
    r")"
)

LISTS = {
    "en": """fuck* motherfuck* shit shits shitty shitting shithead* bullshit* horseshit
        cunt* asshole* arsehole* bitch bitches bitching bitchy slut* whore* wank* twat*
        nigger* nigga niggas faggot* retard retards retarded dickhead* cocksuck* dildo* blowjob*
        handjob* jizz* bollocks tranny trannies kike kikes spic spics""",
    "de": """scheiß* scheiss* fick* gefickt* verfickt* fotze* arschloch* arschlöcher
        hurensohn* hure huren wichser* wichsen schlampe* schwuchtel* neger* missgeburt*
        arschficker* kanake* spasti*""",
    "fr": """putain* merde merdes merdique* enculé* encule* connard* connasse* salope*
        pute putes niquer nique niqué* chier enfoiré* pédé pédés gouine* nègre* branleur*
        bâtard* ntm fdp""",
    "es": """coño joder jodido* jodida* jódete gilipollas hijoputa* hijueputa* cabrón cabrones
        cabrona* puta putas puto putos mierda* maricón* maricones marica pendejo* pendeja*
        verga* chingar* chingad* chingón* culero* follar* hdp""",
    "pt": """porra* caralho* merda* foda fodas foder* fodido* fodida* fode fodeu puta putas
        puto putos buceta* boceta* viado* arrombad* cuzão* piroca* xoxota* xota fdp pqp vsf""",
}

words = set()
entries = LISTS.get(lang, "").split()
exact = {e for e in entries if not e.endswith("*")}
stems = tuple(e[:-1] for e in entries if e.endswith("*"))

morph = None
if lang == "ru":
    try:
        import pymorphy3
        morph = pymorphy3.MorphAnalyzer()
    except ImportError:
        pass

cyrillic = re.compile("[а-яё]")
with open(lexicon, encoding="utf-8") as f:
    for line in f:
        w = line.split("\t", 1)[0]
        if lang in ("ru", "base") and cyrillic.search(w):
            lemmas = {w}
            if morph:
                lemmas.update(p.normal_form for p in morph.parse(w)[:3])
            if any(RU.match(x) for x in lemmas) and "страх" not in w:
                words.add(w)
        elif lang != "ru":
            en = LISTS["en"].split() if lang == "base" else []
            ex = exact | {e for e in en if not e.endswith("*")}
            st = stems + tuple(e[:-1] for e in en if e.endswith("*"))
            if w in ex or (st and w.startswith(st)):
                words.add(w)
sys.stdout.write("".join(f"{w}\n" for w in sorted(words)))
print(f"{len(words)} words", file=sys.stderr)
