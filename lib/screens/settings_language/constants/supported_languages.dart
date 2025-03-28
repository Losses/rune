import 'package:fluent_ui/fluent_ui.dart';

import '../utils/language_option.dart';

const List<LanguageOption> supportedLanguages = [
  LanguageOption(
    title: 'Deutsch',
    sampleText:
        'Alle Menschen sind frei und gleich an Würde und Rechten geboren. Sie sind mit Vernunft und Gewissen begabt und sollen einander im Geist der Brüderlichkeit begegnen.',
    locale: Locale.fromSubtags(languageCode: 'de'),
    experimental: true,
  ),
  LanguageOption(
    title: 'English',
    sampleText:
        'All human beings are born free and equal in dignity and rights. They are endowed with reason and conscience and should act towards one another in a spirit of brotherhood.',
    locale: Locale.fromSubtags(languageCode: 'en'),
  ),
  LanguageOption(
    title: 'Esperanto',
    sampleText:
        'Ĉiuj homoj estas denaske liberaj kaj egalaj laŭ digno kaj rajtoj. Ili posedas racion kaj konsciencon, kaj devus konduti unu al alia en spirito de frateco.',
    // It's a bug of: github.com/flutter/flutter/issues/72422
    // SORRY! I KNOW THIS IS BAD!
    locale: Locale.fromSubtags(languageCode: 'en', countryCode: 'EO'),
    experimental: true,
  ),
  LanguageOption(
    title: 'Français',
    sampleText:
        'Tous les êtres humains naissent libres et égaux en dignité et en droits. Ils sont doués de raison et de conscience et doivent agir les uns envers les autres dans un esprit de fraternité.',
    locale: Locale.fromSubtags(languageCode: 'fr'),
    experimental: true,
  ),
  LanguageOption(
    title: 'Italiano',
    sampleText:
        'Tutti gli esseri umani nascono liberi ed eguali in dignità e diritti. Essi sono dotati di ragione e di coscienza e devono agire gli uni verso gli altri in spirito di fratellanza.',
    locale: Locale.fromSubtags(languageCode: 'it'),
    experimental: true,
  ),
  LanguageOption(
    title: '日本語',
    sampleText:
        'すべての人間は、生まれながらにして自由であり、かつ、尊厳と権利とについて平等である。人間は、理性と良心とを授けられており、互いに同胞の精神をもって行動しなければならない。',
    locale: Locale.fromSubtags(languageCode: 'ja'),
  ),
  LanguageOption(
    title: 'ニホンゴ',
    sampleText:
        'すべてのヒューマンは、ボーンながらにしてフリーであり、かつ、プライドとパワーとについてイコールである。ヒューマンは、ロジックとコンシャスとをギフトされており、ミューチュアルにブラザーシップのスピリットをもってアクトしなければならない。',
    locale: Locale.fromSubtags(languageCode: 'ja', scriptCode: 'Kana'),
  ),
  LanguageOption(
    title: '偽中国語',
    sampleText: '全人間、生自由有、且、尊厳権利付平等有。人間、理性良心授等有、互同胞精神持行動。',
    locale: Locale.fromSubtags(languageCode: 'ja', scriptCode: 'Nise'),
  ),
  LanguageOption(
    title: '한국어',
    sampleText:
        '모든 인간은 태어날 때부터 자유로우며 그 존엄과 권리에 있어 동등하다. 인간은 천부적으로 이성과 양심을 부여받았으며 서로 형제애의 정신으로 행동하여야 한다.',
    locale: Locale.fromSubtags(languageCode: 'ko'),
    experimental: true,
  ),
  LanguageOption(
    title: 'Українська',
    sampleText:
        'Всі люди народжуються вільними і рівними у своїй гідності та правах. Вони наділені розумом і совістю і повинні діяти у відношенні один до одного в дусі братерства.',
    locale: Locale.fromSubtags(languageCode: 'uk'),
    experimental: true,
  ),
  LanguageOption(
    title: '简体中文',
    sampleText: '人人生而自由，在尊严和权利上一律平等。他们禀赋理性和良知，应当以兄弟情谊的精神相对待。',
    locale: Locale.fromSubtags(languageCode: 'zh', scriptCode: 'Hans'),
  ),
  LanguageOption(
    title: '傳統中文',
    sampleText: '人人生而自由，在尊嚴與權利上一律平等。人人皆賦有理性與良知，並應以兄弟關係的精神相對待。',
    locale: Locale.fromSubtags(
        languageCode: 'zh', scriptCode: 'Hant', countryCode: 'TW'),
  ),
  LanguageOption(
    title: '閩南語',
    sampleText: '眾人生來就是自由的，逐家的尊嚴佮權利是平等的。逐家攏有理性佮良心，應該用兄弟姊妹的精神相待。',
    locale: Locale.fromSubtags(
        languageCode: 'zh', scriptCode: 'Hant', countryCode: 'NAN'),
    experimental: true,
  ),
  LanguageOption(
    title: '廣東話',
    sampleText: '人人生而自由，在尊嚴同權利上一律平等。人人具有理性同良心，而且應當以兄弟關係嘅精神相對待。',
    locale: Locale.fromSubtags(
        languageCode: 'zh', scriptCode: 'Hant', countryCode: 'YUE'),
    experimental: true,
  ),
  LanguageOption(
    title: '文言',
    sampleText: '人者，生而平等也；降世以來，便得自由、威武、平等、心觀，彼此皆如手足。',
    locale: Locale.fromSubtags(
        languageCode: 'zh', scriptCode: 'Hant', countryCode: 'LZH'),
  ),
];
