//! The keyboard's own words — settings, the guide, strip prompts, notices —
//! in the phone's language: Russian, English, German, French, Spanish or
//! Portuguese; English for any other.

use crate::layout::Lang;

/// The interface language for a locale code (`de`, `pt`, `uk`…).
pub fn ui_lang(locale: &str) -> Lang {
    match locale {
        "ru" | "uk" | "be" | "kk" => Lang::Ru,
        other => Lang::from_code(other).unwrap_or(Lang::En),
    }
}

/// The text for `key` in `lang`; `{}` in it stands for a word. Unknown keys
/// come back empty.
pub fn t(lang: Lang, key: &str) -> &'static str {
    let i = match lang {
        Lang::Ru => 0,
        Lang::En => 1,
        Lang::De => 2,
        Lang::Fr => 3,
        Lang::Es => 4,
        Lang::Pt => 5,
    };
    TEXTS
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, texts)| texts[i])
}

/// The texts the settings screen (Java) shows itself, as `key\ttext`.
pub fn screen_texts(lang: Lang) -> Vec<String> {
    TEXTS
        .iter()
        .filter(|(k, _)| k.starts_with("ui."))
        .map(|(k, _)| format!("{k}\t{}", t(lang, k)))
        .collect()
}

/// Key → [ru, en, de, fr, es, pt].
const TEXTS: &[(&str, [&str; 6])] = &[
    // --- the strip: the user dictionary ---
    ("dict.add", ["＋ в словарь: ", "＋ to dictionary: ", "＋ ins Wörterbuch: ", "＋ au dictionnaire : ", "＋ al diccionario: ", "＋ ao dicionário: "]),
    ("dict.remove", ["− из словаря: ", "− from dictionary: ", "− aus dem Wörterbuch: ", "− du dictionnaire : ", "− del diccionario: ", "− do dicionário: "]),
    ("dict.as_name", [" (имя)", " (name)", " (Name)", " (nom propre)", " (nombre propio)", " (nome próprio)"]),
    ("dict.already", ["«{}» уже в словаре", "“{}” is already in the dictionary", "„{}“ ist schon im Wörterbuch", "« {} » est déjà dans le dictionnaire", "«{}» ya está en el diccionario", "“{}” já está no dicionário"]),
    ("dict.added", ["«{}» — в словаре", "“{}” added to the dictionary", "„{}“ ist jetzt im Wörterbuch", "« {} » ajouté au dictionnaire", "«{}» añadido al diccionario", "“{}” adicionado ao dicionário"]),
    ("dict.removed", ["«{}» убрано из словаря", "“{}” removed from the dictionary", "„{}“ aus dem Wörterbuch entfernt", "« {} » retiré du dictionnaire", "«{}» eliminado del diccionario", "“{}” removido do dicionário"]),
    // --- the strip: the calibration ---
    ("cal.cancelled", ["Калибровка отменена", "Calibration cancelled", "Kalibrierung abgebrochen", "Calibrage annulé", "Calibración cancelada", "Calibração cancelada"]),
    ("cal.saved", ["Калибровка сохранена", "Calibration saved", "Kalibrierung gespeichert", "Calibrage enregistré", "Calibración guardada", "Calibração salva"]),
    ("cal.more", ["Мало — закрасьте побольше", "Too little — paint more", "Zu wenig — mehr ausmalen", "Trop peu — coloriez davantage", "Muy poco — pinta más", "Pouco — pinte mais"]),
    ("cal.area", ["{}: закрасьте, куда удобно достаёт палец", "{}: paint where the thumb comfortably reaches", "{}: ausmalen, wohin der Daumen bequem reicht", "{} : coloriez là où le pouce atteint sans effort", "{}: pinta hasta donde llega cómodo el pulgar", "{}: pinte onde o polegar alcança com conforto"]),
    ("cal.done", ["Готово", "Done", "Fertig", "Terminé", "Listo", "Pronto"]),
    ("emoji.search", ["Поиск эмодзи", "Search emoji", "Emoji suchen", "Chercher un emoji", "Buscar emoji", "Buscar emoji"]),
    ("arc.need_calibration", ["Сначала калибровка хвата — в настройках", "Calibrate the grip first — in the settings", "Erst den Griff kalibrieren — in den Einstellungen", "Calibrez d'abord la prise — dans les réglages", "Primero calibra el agarre — en los ajustes", "Primeiro calibre a pegada — nos ajustes"]),
    ("cal.skip", ["Пропустить", "Skip", "Überspringen", "Passer", "Omitir", "Pular"]),
    ("cal.cancel", ["Отмена", "Cancel", "Abbrechen", "Annuler", "Cancelar", "Cancelar"]),
    ("grip.left", ["Левой", "Left hand", "Linke Hand", "Main gauche", "Mano izquierda", "Mão esquerda"]),
    ("grip.right", ["Правой", "Right hand", "Rechte Hand", "Main droite", "Mano derecha", "Mão direita"]),
    ("grip.both", ["Двумя", "Both hands", "Beide Hände", "Deux mains", "Dos manos", "Duas mãos"]),
    ("grip.finger", ["Одним пальцем", "One finger", "Ein Finger", "Un doigt", "Un dedo", "Um dedo"]),
    // --- settings ---
    ("set.languages", [
        "Языки — в этом порядке их перебирают кнопка языка и свайп по пробелу",
        "Languages — in the order the language key and a swipe on the space bar go through them",
        "Sprachen — in dieser Reihenfolge wechseln Sprachtaste und Wischen über die Leertaste",
        "Langues — dans l'ordre où la touche de langue et un glissement sur l'espace les parcourent",
        "Idiomas — en el orden en que los recorren la tecla de idioma y el deslizamiento sobre el espacio",
        "Idiomas — na ordem em que a tecla de idioma e o deslize no espaço passam por eles",
    ]),
    ("set.theme", ["Тема", "Theme", "Design", "Thème", "Tema", "Tema"]),
    ("set.theme.system", ["Как в системе", "As the system", "Wie das System", "Comme le système", "Como el sistema", "Como o sistema"]),
    ("set.theme.light", ["Светлая", "Light", "Hell", "Clair", "Claro", "Claro"]),
    ("set.theme.dark", ["Тёмная", "Dark", "Dunkel", "Sombre", "Oscuro", "Escuro"]),
    ("set.borderless", ["Клавиши без заливки: только буквы на фоне", "Keys without a fill: just the letters on the background", "Tasten ohne Füllung: nur die Buchstaben auf dem Hintergrund", "Touches sans fond : seulement les lettres sur l'arrière-plan", "Teclas sin relleno: solo las letras sobre el fondo", "Teclas sem preenchimento: só as letras sobre o fundo"]),
    ("set.emoji_on_enter", ["Эмодзи — на зажатие Enter (вместо запятой)", "Emoji on holding Enter (instead of the comma)", "Emoji durch Halten von Enter (statt des Kommas)", "Emoji en maintenant Entrée (au lieu de la virgule)", "Emoji al mantener Intro (en lugar de la coma)", "Emoji ao segurar Enter (em vez da vírgula)"]),
    ("set.theme.black", ["Чёрная (AMOLED)", "Black (AMOLED)", "Schwarz (AMOLED)", "Noir (AMOLED)", "Negro (AMOLED)", "Preto (AMOLED)"]),
    ("set.wallpaper_colors", [
        "Цвета из обоев (Android 12+)",
        "Colors from the wallpaper (Android 12+)",
        "Farben aus dem Hintergrundbild (Android 12+)",
        "Couleurs du fond d'écran (Android 12+)",
        "Colores del fondo de pantalla (Android 12+)",
        "Cores do papel de parede (Android 12+)",
    ]),
    ("set.autocorrect", ["Исправлять слово при пробеле", "Correct the word on space", "Wort bei Leertaste korrigieren", "Corriger le mot à l'espace", "Corregir la palabra al pulsar espacio", "Corrigir a palavra ao apertar espaço"]),
    ("set.strength", ["Смелость исправлений", "How boldly to correct", "Wie mutig korrigieren", "Audace des corrections", "Audacia de las correcciones", "Ousadia das correções"]),
    ("set.strength.careful", ["Осторожно", "Carefully", "Vorsichtig", "Prudemment", "Con cuidado", "Com cuidado"]),
    ("set.strength.normal", ["Обычно", "Normally", "Normal", "Normalement", "Normal", "Normal"]),
    ("set.strength.bold", ["Смело", "Boldly", "Mutig", "Hardiment", "Con audacia", "Com ousadia"]),
    ("set.strength.always", ["Всегда", "Always", "Immer", "Toujours", "Siempre", "Sempre"]),
    ("set.block_offensive", [
        "Не подсказывать мат и не исправлять на него (набранное по буквам остаётся)",
        "Don't suggest obscene words or correct to them (typed letter by letter they stay)",
        "Keine Schimpfwörter vorschlagen oder dazu korrigieren (buchstabenweise getippt bleiben sie)",
        "Ne pas proposer de gros mots ni corriger vers eux (tapés lettre par lettre, ils restent)",
        "No sugerir palabrotas ni corregir a ellas (escritas letra a letra se quedan)",
        "Não sugerir palavrões nem corrigir para eles (digitados letra a letra, ficam)",
    ]),
    ("set.live_correction", [
        "Показывать исправление сразу в тексте (эксперимент)",
        "Show the correction in the text right away (experimental)",
        "Korrektur sofort im Text zeigen (experimentell)",
        "Afficher la correction directement dans le texte (expérimental)",
        "Mostrar la corrección directamente en el texto (experimental)",
        "Mostrar a correção direto no texto (experimental)",
    ]),
    ("set.strip", ["Полоса подсказок", "Suggestion strip", "Vorschlagsleiste", "Barre de suggestions", "Barra de sugerencias", "Barra de sugestões"]),
    ("set.strip.two", ["Моё слово и исправление", "My word and the correction", "Mein Wort und die Korrektur", "Mon mot et la correction", "Mi palabra y la corrección", "Minha palavra e a correção"]),
    ("set.strip.full", ["Три подсказки", "Three suggestions", "Drei Vorschläge", "Trois suggestions", "Tres sugerencias", "Três sugestões"]),
    ("set.strip.off", ["Нет", "None", "Keine", "Aucune", "Ninguna", "Nenhuma"]),
    ("set.auto_caps", ["Заглавная в начале предложения", "Capital at the start of a sentence", "Großbuchstabe am Satzanfang", "Majuscule en début de phrase", "Mayúscula al principio de la frase", "Maiúscula no início da frase"]),
    ("set.haptic", ["Вибрация при нажатии", "Vibrate on keypress", "Vibration bei Tastendruck", "Vibrer à chaque touche", "Vibrar al pulsar", "Vibrar ao tocar"]),
    ("set.lang_switch", ["Смена языка", "Switching languages", "Sprachwechsel", "Changement de langue", "Cambio de idioma", "Troca de idioma"]),
    ("set.lang_switch.both", ["Кнопка и свайп по пробелу", "Key and swipe on the space bar", "Taste und Wischen über die Leertaste", "Touche et glissement sur l'espace", "Tecla y deslizamiento sobre el espacio", "Tecla e deslize no espaço"]),
    ("set.lang_switch.key", ["Только кнопка", "Key only", "Nur Taste", "Touche seulement", "Solo tecla", "Só tecla"]),
    ("set.lang_switch.swipe", ["Только свайп", "Swipe only", "Nur Wischen", "Glissement seulement", "Solo deslizamiento", "Só deslize"]),
    ("set.lang_from_text", [
        "Переключать язык по слову у курсора (английское слово в русском тексте — английский)",
        "Switch to the language of the word at the cursor (an English word in a Russian text — English)",
        "Zur Sprache des Wortes am Cursor wechseln (ein englisches Wort in deutschem Text — Englisch)",
        "Passer à la langue du mot sous le curseur (un mot anglais dans un texte français — anglais)",
        "Cambiar al idioma de la palabra junto al cursor (una palabra inglesa en un texto español — inglés)",
        "Mudar para o idioma da palavra junto ao cursor (uma palavra inglesa num texto em português — inglês)",
    ]),
    ("set.one_row", [
        "Раскладка в один ряд (компактная, как Minuum)",
        "One-row layout (compact, like Minuum)",
        "Einzeilige Tastatur (kompakt, wie Minuum)",
        "Clavier sur une ligne (compact, comme Minuum)",
        "Teclado de una fila (compacto, como Minuum)",
        "Teclado de uma linha (compacto, como o Minuum)",
    ]),
    ("set.touch_points", [
        "Учитывать точку касания, а не только букву",
        "Use where the finger touched, not just the key",
        "Berührungspunkt berücksichtigen, nicht nur die Taste",
        "Tenir compte du point de contact, pas seulement de la touche",
        "Tener en cuenta el punto de contacto, no solo la tecla",
        "Considerar o ponto de toque, não só a tecla",
    ]),
    ("set.height", ["Высота клавиатуры", "Keyboard height", "Tastaturhöhe", "Hauteur du clavier", "Altura del teclado", "Altura do teclado"]),
    ("set.height.90", ["Ниже", "Lower", "Niedriger", "Plus bas", "Más baja", "Mais baixo"]),
    ("set.height.100", ["Обычная", "Normal", "Normal", "Normale", "Normal", "Normal"]),
    ("set.height.115", ["Выше", "Taller", "Höher", "Plus haut", "Más alta", "Mais alto"]),
    ("set.gestures", [
        "Набор свайпом: вести палец по буквам, не отрывая",
        "Swipe typing: slide the finger over the letters without lifting it",
        "Wischen: den Finger über die Buchstaben ziehen, ohne ihn abzusetzen",
        "Saisie gestuelle : glisser le doigt sur les lettres sans le lever",
        "Escritura deslizante: pasar el dedo por las letras sin levantarlo",
        "Digitação por deslize: passar o dedo pelas letras sem levantar",
    ]),
    ("set.gesture_pace", [
        "Свайп: учитывать, где палец замедлился или задержался",
        "Swipe: take into account where the finger slowed down or paused",
        "Wischen: berücksichtigen, wo der Finger langsamer wurde oder verweilte",
        "Geste : tenir compte des endroits où le doigt a ralenti ou s'est arrêté",
        "Deslizar: tener en cuenta dónde el dedo frenó o se detuvo",
        "Deslize: considerar onde o dedo desacelerou ou parou",
    ]),
    ("set.clipboard", [
        "Буфер обмена: предлагать вставить только что скопированное; недавние копии — во вкладке 📋 у эмодзи (только в памяти)",
        "Clipboard: offer to paste what was just copied; recent copies in the 📋 tab by the emoji (in memory only)",
        "Zwischenablage: gerade Kopiertes zum Einfügen anbieten; letzte Kopien im 📋-Tab bei den Emoji (nur im Speicher)",
        "Presse-papiers : proposer de coller ce qui vient d'être copié ; copies récentes dans l'onglet 📋 des emoji (en mémoire seulement)",
        "Portapapeles: ofrecer pegar lo que se acaba de copiar; copias recientes en la pestaña 📋 de los emojis (solo en memoria)",
        "Área de transferência: oferecer colar o que acabou de ser copiado; cópias recentes na aba 📋 dos emojis (só na memória)",
    ]),
    ("set.auto_commas", [
        "Запятые: ставить сами, где почти наверняка нужны (перед «что», «который», «но»…); ⌫ сразу после убирает",
        "Commas: put in by themselves where almost surely due; ⌫ right after takes one out",
        "Kommas: von selbst setzen, wo sie fast sicher hingehören; ⌫ direkt danach nimmt es weg",
        "Virgules : les mettre d'elles-mêmes là où elles sont presque sûres ; ⌫ juste après l'enlève",
        "Comas: ponerlas solas donde casi seguro van; ⌫ justo después la quita",
        "Vírgulas: colocar sozinhas onde quase certamente vão; ⌫ logo depois tira",
    ]),
    ("set.yo", [
        "Ё: писать «ещё», «пошёл», «её», когда набрано через е",
        "Ё (Russian): write «ещё», «пошёл», «её» when typed with е",
        "Ё (Russisch): «ещё», «пошёл», «её» schreiben, wenn mit е getippt",
        "Ё (russe) : écrire «ещё», «пошёл», «её» quand tapé avec е",
        "Ё (ruso): escribir «ещё», «пошёл», «её» cuando se teclea con е",
        "Ё (russo): escrever «ещё», «пошёл», «её» quando digitado com е",
    ]),
    ("panel.clips_empty", [
        "Здесь будет то, что вы скопируете, пока клавиатура работает. Нигде не сохраняется.",
        "What you copy while the keyboard runs shows up here. It is never saved.",
        "Was du kopierst, während die Tastatur läuft, erscheint hier. Es wird nirgends gespeichert.",
        "Ce que vous copiez pendant que le clavier tourne s'affiche ici. Rien n'est enregistré.",
        "Aquí aparece lo que copies mientras el teclado funciona. No se guarda en ningún sitio.",
        "O que você copiar enquanto o teclado funciona aparece aqui. Nada é salvo.",
    ]),
    ("panel.recent_empty", [
        "Здесь будут эмодзи, которые вы вставляли",
        "The emoji you use show up here",
        "Hier erscheinen die Emoji, die du benutzt",
        "Les emoji que vous utilisez s'affichent ici",
        "Aquí aparecen los emojis que usas",
        "Os emojis que você usa aparecem aqui",
    ]),
    ("set.calibrate_left", ["[I ] Левая рука: область пальца и фраза", "[I ] Left hand: the thumb's area and a phrase", "[I ] Linke Hand: Daumenbereich und ein Satz", "[I ] Main gauche : zone du pouce et une phrase", "[I ] Mano izquierda: zona del pulgar y una frase", "[I ] Mão esquerda: área do polegar e uma frase"]),
    ("set.calibrate_right", ["[ I] Правая рука: область пальца и фраза", "[ I] Right hand: the thumb's area and a phrase", "[ I] Rechte Hand: Daumenbereich und ein Satz", "[ I] Main droite : zone du pouce et une phrase", "[ I] Mano derecha: zona del pulgar y una frase", "[ I] Mão direita: área do polegar e uma frase"]),
    ("set.calibrate_both", ["[II] Двумя руками: фраза", "[II] Both hands: a phrase", "[II] Beide Hände: ein Satz", "[II] Deux mains : une phrase", "[II] Dos manos: una frase", "[II] Duas mãos: uma frase"]),
    ("set.calibrate_finger", ["[•] Одним пальцем, телефон на столе: фраза", "[•] One finger, the phone on a table: a phrase", "[•] Ein Finger, Telefon auf dem Tisch: ein Satz", "[•] Un doigt, téléphone sur la table : une phrase", "[•] Un dedo, teléfono en la mesa: una frase", "[•] Um dedo, telefone na mesa: uma frase"]),
    ("set.arc_layout", [
        "Одной рукой: клавиатура сама подстраивается под левый или правый большой палец по наклону телефона (нужна калибровка хвата)",
        "One-handed: the keyboard follows the left or right thumb by the phone's tilt (needs the grip calibration)",
        "Einhändig: die Tastatur folgt dem linken oder rechten Daumen nach der Neigung des Telefons (braucht die Griff-Kalibrierung)",
        "À une main : le clavier suit le pouce gauche ou droit selon l'inclinaison du téléphone (calibrage de la prise requis)",
        "A una mano: el teclado sigue al pulgar izquierdo o derecho según la inclinación del teléfono (requiere calibrar el agarre)",
        "Com uma mão: o teclado segue o polegar esquerdo ou direito pela inclinação do telefone (requer calibrar a pegada)",
    ]),
    ("set.grip_undo", ["Откатить", "Undo", "Zurück", "Annuler", "Deshacer", "Desfazer"]),
    ("set.grip_indicator", [
        "Показывать хват: [I ] левая, [ I] правая, [II] две руки, [•] один палец",
        "Show the grip: [I ] left, [ I] right, [II] both hands, [•] one finger",
        "Griff anzeigen: [I ] links, [ I] rechts, [II] beide Hände, [•] ein Finger",
        "Afficher la prise : [I ] gauche, [ I] droite, [II] deux mains, [•] un doigt",
        "Mostrar el agarre: [I ] izquierda, [ I] derecha, [II] dos manos, [•] un dedo",
        "Mostrar a pegada: [I ] esquerda, [ I] direita, [II] duas mãos, [•] um dedo",
    ]),
    ("set.grip_offsets", [
        "Поправлять касания по калибровке",
        "Adjust taps by the calibration",
        "Berührungen nach der Kalibrierung korrigieren",
        "Ajuster les touchers selon le calibrage",
        "Ajustar los toques según la calibración",
        "Ajustar os toques pela calibração",
    ]),
    ("set.grip_reset", ["Сбросить всю калибровку", "Reset the whole calibration", "Die ganze Kalibrierung zurücksetzen", "Réinitialiser tout le calibrage", "Restablecer toda la calibración", "Redefinir toda a calibração"]),
    ("set.debug_log", [
        "Для разработчиков: журнал набора в поле «Попробовать» (typing-log.jsonl)",
        "For developers: typing log in the “Try it” field (typing-log.jsonl)",
        "Für Entwickler: Tipp-Protokoll im Testfeld (typing-log.jsonl)",
        "Pour les développeurs : journal de saisie dans le champ d'essai (typing-log.jsonl)",
        "Para desarrolladores: registro de escritura en el campo de prueba (typing-log.jsonl)",
        "Para desenvolvedores: registro de digitação no campo de teste (typing-log.jsonl)",
    ]),
    // --- the settings screen's own texts (Java) ---
    ("ui.enable", ["Включить Rust KB в системе", "Enable Rust KB in the system", "Rust KB im System aktivieren", "Activer Rust KB dans le système", "Activar Rust KB en el sistema", "Ativar o Rust KB no sistema"]),
    ("ui.pick", ["Выбрать клавиатуру", "Choose the keyboard", "Tastatur wählen", "Choisir le clavier", "Elegir el teclado", "Escolher o teclado"]),
    ("ui.try", ["Попробовать клавиатуру здесь", "Try the keyboard here", "Tastatur hier ausprobieren", "Essayer le clavier ici", "Probar el teclado aquí", "Experimentar o teclado aqui"]),
    ("ui.github", [
        "Исходный код и поддержка проекта — GitHub",
        "Source code and supporting the project — GitHub",
        "Quellcode und das Projekt unterstützen — GitHub",
        "Code source et soutenir le projet — GitHub",
        "Código fuente y apoyar el proyecto — GitHub",
        "Código-fonte e apoiar o projeto — GitHub",
    ]),
    ("ui.licenses", ["Лицензии и источники данных", "Licenses and data sources", "Lizenzen und Datenquellen", "Licences et sources des données", "Licencias y fuentes de datos", "Licenças e fontes de dados"]),
    ("ui.guide_button", ["Как пользоваться", "How to use it", "So geht's", "Mode d'emploi", "Cómo se usa", "Como usar"]),
    ("ui.guide_ok", ["Понятно", "Got it", "Verstanden", "Compris", "Entendido", "Entendi"]),
    ("ui.guide", [
        "Как пользоваться\n\n\
• Включите Rust KB и выберите её — кнопки ниже.\n\
• Свайп: ведите палец по буквам слова, не отрывая, — слово напишется целиком. ⌫ сразу после — стирает его.\n\
• Язык: смахните пробел влево или вправо (или нажмите кнопку языка). Какие языки и в каком порядке — в настройках ниже.\n\
• Долгое нажатие на пробел — другая клавиатура или эти настройки.\n\
• ⌫ сразу после исправления возвращает то, что вы написали. Свайп вверх по пробелу — следующий вариант слова.\n\
• Долгое нажатие на букву — цифра, ё и буквы с акцентами. Долгое нажатие на запятую — эмодзи и недавно скопированное (📋).\n\
• Выделите новое слово — клавиатура предложит добавить его в словарь.\n\
• Клавиатура работает без интернета и ничего не отправляет и не запоминает о вас.",
        "How to use it\n\n\
• Enable Rust KB and choose it — the buttons below.\n\
• Swipe: slide your finger over the letters of a word without lifting it — the whole word is typed. ⌫ right after erases it.\n\
• Language: swipe the space bar left or right (or tap the language key). Which languages and in what order — in the settings below.\n\
• Hold the space bar for another keyboard or these settings.\n\
• ⌫ right after a correction brings back what you typed. Swipe up on the space bar for the next likeliest word.\n\
• Hold a letter for its digit and accented letters. Hold the comma for emoji and what you copied lately (📋).\n\
• Select a new word and the keyboard offers to add it to the dictionary.\n\
• The keyboard works offline and never sends or remembers anything about you.",
        "So geht's\n\n\
• Rust KB aktivieren und auswählen — mit den Knöpfen unten.\n\
• Wischen: den Finger über die Buchstaben eines Wortes ziehen, ohne ihn abzusetzen — das ganze Wort wird geschrieben. ⌫ direkt danach löscht es.\n\
• Sprache: die Leertaste nach links oder rechts wischen (oder die Sprachtaste tippen). Welche Sprachen in welcher Reihenfolge — unten in den Einstellungen.\n\
• Leertaste gedrückt halten: eine andere Tastatur oder diese Einstellungen.\n\
• ⌫ direkt nach einer Korrektur holt zurück, was du getippt hast. Nach oben über die Leertaste wischen: das nächstwahrscheinliche Wort.\n\
• Einen Buchstaben gedrückt halten: Ziffer und Buchstaben mit Akzent. Das Komma gedrückt halten: Emoji und zuletzt Kopiertes (📋).\n\
• Ein neues Wort markieren — die Tastatur bietet an, es ins Wörterbuch aufzunehmen.\n\
• Die Tastatur arbeitet offline und sendet oder speichert nichts über dich.",
        "Mode d'emploi\n\n\
• Activez Rust KB et choisissez-le — boutons ci-dessous.\n\
• Glisser : faites glisser le doigt sur les lettres d'un mot sans le lever — le mot entier s'écrit. ⌫ juste après l'efface.\n\
• Langue : glissez sur la barre d'espace vers la gauche ou la droite (ou touchez la touche de langue). Quelles langues et dans quel ordre — dans les réglages ci-dessous.\n\
• Maintenez l'espace : un autre clavier ou ces réglages.\n\
• ⌫ juste après une correction rend ce que vous aviez tapé. Glissez vers le haut sur l'espace : le mot suivant le plus probable.\n\
• Maintenez une lettre : son chiffre et ses lettres accentuées. Maintenez la virgule : les emoji et ce que vous avez copié récemment (📋).\n\
• Sélectionnez un nouveau mot : le clavier propose de l'ajouter au dictionnaire.\n\
• Le clavier fonctionne hors ligne et n'envoie ni ne retient rien sur vous.",
        "Cómo se usa\n\n\
• Activa Rust KB y elígelo con los botones de abajo.\n\
• Deslizar: pasa el dedo por las letras de una palabra sin levantarlo y se escribe entera. ⌫ justo después la borra.\n\
• Idioma: desliza la barra espaciadora a la izquierda o a la derecha (o toca la tecla de idioma). Qué idiomas y en qué orden, en los ajustes de abajo.\n\
• Mantén pulsado el espacio: otro teclado o estos ajustes.\n\
• ⌫ justo después de una corrección recupera lo que escribiste. Desliza hacia arriba sobre el espacio: la siguiente palabra más probable.\n\
• Mantén pulsada una letra: su número y sus letras con tilde. Mantén pulsada la coma: emojis y lo que copiaste hace poco (📋).\n\
• Selecciona una palabra nueva y el teclado te ofrece añadirla al diccionario.\n\
• El teclado funciona sin conexión y no envía ni recuerda nada sobre ti.",
        "Como usar\n\n\
• Ative o Rust KB e escolha-o — botões abaixo.\n\
• Deslize: passe o dedo pelas letras de uma palavra sem levantar — a palavra inteira é escrita. ⌫ logo depois a apaga.\n\
• Idioma: deslize a barra de espaço para a esquerda ou a direita (ou toque na tecla de idioma). Quais idiomas e em que ordem — nos ajustes abaixo.\n\
• Segure o espaço: outro teclado ou estes ajustes.\n\
• ⌫ logo depois de uma correção traz de volta o que você digitou. Deslize para cima no espaço: a próxima palavra mais provável.\n\
• Segure uma letra: o número e as letras com acento. Segure a vírgula: emojis e o que você copiou há pouco (📋).\n\
• Selecione uma palavra nova e o teclado oferece adicioná-la ao dicionário.\n\
• O teclado funciona offline e não envia nem guarda nada sobre você.",
    ]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_is_there_in_every_language() {
        for (key, texts) in TEXTS {
            for (i, text) in texts.iter().enumerate() {
                assert!(!text.is_empty(), "{key} [{i}]");
                assert_eq!(
                    text.matches("{}").count(),
                    texts[0].matches("{}").count(),
                    "{key} [{i}]: the word's place"
                );
            }
        }
        let mut keys: Vec<&str> = TEXTS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), TEXTS.len(), "a key twice");
    }

    #[test]
    fn the_phone_language_picks_the_texts() {
        assert_eq!(ui_lang("uk"), Lang::Ru);
        assert_eq!(ui_lang("pt"), Lang::Pt);
        assert_eq!(ui_lang("ja"), Lang::En);
        assert_eq!(t(Lang::De, "cal.skip"), "Überspringen");
        assert_eq!(t(Lang::De, "no.such.key"), "");
    }
}
