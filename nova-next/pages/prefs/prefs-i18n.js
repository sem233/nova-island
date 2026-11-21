// The Settings window in Arabic (1.8): Settings › General › Language. The page stays as 1.7 wrote
// it, in English; this layer puts the Arabic text over it, right to left, and follows every text
// the page writes later (statuses, the stepper values, the plugin list). English comes back the
// same way, without reloading. Names stay as they are: Nova Island, Ko-fi, InstaPay, GitHub.
(() => {
  const AR = {
    // window and sidebar
    'Nova Island Settings': 'إعدادات Nova Island',
    'General': 'عام',
    'Island': 'الجزيرة',
    // Widgets (1.8)
    'Widgets': 'الويدجات',
    'Desktop widgets': 'ويدجات سطح المكتب',
    'Show widgets on the desktop': 'إظهار الويدجات على سطح المكتب',
    'Glass cards under your windows, drawn by Nova itself. Drag them anywhere; near each other they line up': 'بطاقات زجاجية تحت نوافذك يرسمها Nova بنفسه. اسحبها إلى أي مكان، وحين تقترب من بعضها تصطف تلقائيًا',
    'Layout': 'الترتيب',
    'Widgets in the layout': 'ويدجات الترتيب',
    'More widgets': 'ويدجات أخرى',
    'Put them back in place': 'إعادتها إلى مكانها',
    'The layout to the top-right corner of the screen': 'يعود الترتيب إلى أعلى يمين الشاشة',
    'Reset places': 'إعادة الأماكن',
    'Click a widget below to put it on your desktop. Drag it there to place it; right-click it for more': 'اضغط على أي ويدجت بالأسفل ليظهر على سطح المكتب، واسحبه هناك لتضعه حيث تريد، وانقر عليه بالزر الأيمن لخيارات أكثر',
    'On your desktop': 'على سطح المكتب',
    'Add': 'إضافة',
    'Update ready': 'التحديث جاهز',
    'A new version of Nova Island is ready to install': 'إصدار جديد من Nova Island جاهز للتثبيت',
    'Update now': 'حدّث الآن',
    'Check for updates': 'البحث عن تحديثات',
    'Check now': 'ابحث الآن',
    'Check fork updates': 'تحديثات المستودع (sem233)',
    'Check fork': 'فحص المستودع',
    'Checking…': 'جارٍ البحث…',
    'Version': 'الإصدار',
    'You have the latest version': 'لديك أحدث إصدار',
    'A new version is ready': 'إصدار جديد جاهز',
    "Couldn't check. Check your internet connection and try again": 'تعذّر البحث. تأكد من اتصالك بالإنترنت وحاول مرة أخرى',
    'Repository is private or release manifest is missing (HTTP 404)': 'المستودع خاص أو ملف التحديث غير منشور (HTTP 404)',
    'No release manifest found on upstream (HTTP 404)': 'ملف التحديث غير موجود في المستودع الأصلي (HTTP 404)',
    'Downloaded file verification failed (SHA-512 mismatch)': 'فشل التحقق من ملف التحديث (عدم تطابق SHA-512)',
    'Release manifest could not be read': 'تعذّر قراءة بيانات الإصدار (latest.yml)',
    'Updating…': 'جارٍ التحديث…',
    'Coming soon': 'قريبًا',
    'Today': 'اليوم',
    'Add a widget': 'أضف ويدجت',
    'Add a ready group': 'أضف مجموعة جاهزة',
    'On desktop': 'على سطح المكتب',
    'Ungroup': 'فك المجموعة',
    'Group': 'مجموعة',
    'Nothing on your desktop yet. Add a widget below': 'لا شيء على سطح المكتب بعد. أضف ويدجت من الأسفل',
    'Every widget to the first clear place from the top-right corner of the screen': 'تعود كل الويدجات إلى أول مكان خالٍ من أعلى يمين الشاشة',
    'Work': 'العمل',
    'Dhikr': 'الذكر',
    'Full board': 'لوحة كاملة',
    'PC': 'الجهاز',
    'Choose a layout or widgets to see them here': 'اختر ترتيبًا أو ويدجات لتظهر هنا',
    'Preview': 'معاينة',
    'Wide': 'عريض',
    'Small': 'صغير',
    'Empty': 'فارغ',
    'No layout': 'بلا ترتيب',
    '1 wide, 2 small': 'عريض واثنان صغيران',
    '1 wide, 4 small': 'عريض وأربعة صغيرة',
    '2 wide, 2 small': 'عريضان واثنان صغيران',
    '2 wide, 4 small': 'عريضان وأربعة صغيرة',
    '4 small': 'أربعة صغيرة',
    '3 wide': 'ثلاثة عريضة',
    'Next prayer': 'الصلاة القادمة',
    'Five prayers': 'الصلوات الخمس',
    "Sun's path": 'مسار الشمس',
    'Day name': 'اسم اليوم',
    'Day and dates': 'اليوم والتاريخان',
    'Month': 'التقويم الشهري',
    "Today's tasks": 'مهام اليوم',
    'Tasbih': 'السبحة',
    'Moon': 'طور القمر',
    'Focus timer': 'مؤقت التركيز',
    'Water bottle': 'زجاجة الماء',
    'PC pulse': 'نبض الجهاز',
    'Themes': 'المظاهر',
    'Companion': 'الرفيق',
    'Prayer & Dhikr': 'الصلاة والذكر',
    'Alerts & Privacy': 'التنبيهات والخصوصية',
    'Connections': 'الاتصالات',
    'Plugins': 'الإضافات',
    'Support': 'الدعم',
    'More by Nova': 'المزيد من Nova',
    'Nova on Facebook': 'Nova على فيسبوك',
    'Facebook': 'فيسبوك',
    'Telegram': 'تيليجرام',
    'opens the island': 'لفتح الجزيرة',
    'No matching settings': 'لا توجد إعدادات مطابقة',
    'Minimize': 'تصغير',
    'Close': 'إغلاق',
    'Pages': 'الصفحات',
    'Search': 'بحث',
    'Saved': 'تم الحفظ',
    // General
    'Language': 'اللغة',
    'Show island': 'إظهار الجزيرة',
    'Start with Windows': 'التشغيل مع Windows',
    'Managed by Windows for the Store version': 'يتحكم فيه Windows في نسخة المتجر',
    'Open startup apps': 'فتح تطبيقات بدء التشغيل',
    'Orientation': 'الاتجاه',
    'Time format': 'تنسيق الوقت',
    '12-hour': '12 ساعة',
    '24-hour': '24 ساعة',
    'Horizontal': 'أفقي',
    'Vertical': 'رأسي',
    'In the browser': 'في المتصفح',
    'Address bar': 'شريط العنوان',
    'Stay in place': 'البقاء في مكانها',
    'Thin line': 'خط رفيع',
    'Show on the taskbar': 'العرض في شريط المهام',
    "Nova's eyes and what matters now on the taskbar, beside the weather: media, the next prayer, performance": 'عينا Nova وأهم ما يجري الآن في شريط المهام بجوار الطقس: الوسائط، والصلاة القادمة، والأداء',
    'A thin line over the tabs; point at it to open the island': 'خط رفيع فوق علامات التبويب؛ أشِر إليه لتُفتح الجزيرة',
    'Moves into the address bar, from its default place at the top': 'تنتقل إلى شريط العنوان من مكانها المعتاد في الأعلى',
    'Stays where it is; clicks go to the tabs below': 'تبقى في مكانها، وتصل النقرات إلى علامات التبويب أسفلها',
    'Hide in full screen': 'الإخفاء في وضع ملء الشاشة',
    'Auto-update': 'التحديث التلقائي',
    'Position': 'الموضع',
    'Reset': 'إعادة الضبط',
    'Performance': 'الأداء',
    'Remove blur': 'إزالة التمويه',
    'A solid island; the screen is not captured': 'جزيرة بلون ثابت، ولا تُلتقط الشاشة',
    'Remove animations': 'إزالة الحركات',
    'Freeze glass while closed': 'تثبيت الزجاج أثناء الإغلاق',
    'The glass refreshes only while the island is open': 'لا يتجدد الزجاج إلا والجزيرة مفتوحة',
    'Battery saver': 'توفير البطارية',
    'Turns all three on when the battery runs low': 'يشغّل الخيارات الثلاثة عند انخفاض البطارية',
    'Below': 'أقل من',
    'No battery': 'لا توجد بطارية',
    'Location': 'الموقع',
    'City': 'المدينة',
    'Auto': 'تلقائي',
    'Use auto': 'استخدام التلقائي',
    'Search for a city': 'ابحث عن مدينة',
    'Searching…': 'جارٍ البحث…',
    'Search failed. Check your connection.': 'تعذّر البحث. تحقق من اتصالك.',
    'No results': 'لا توجد نتائج',
    // Island
    'Tabs': 'علامات التبويب',
    'Media': 'الوسائط',
    'Apps': 'التطبيقات',
    'Shelf': 'الرف',
    'Prayer': 'الصلاة',
    'Timer': 'المؤقت',
    'System': 'النظام',
    'Keep at least one tab': 'أبقِ علامة تبويب واحدة على الأقل',
    'Collapsed': 'وهي مغلقة',
    'Clock & weather': 'الساعة والطقس',
    'CPU & memory': 'المعالج والذاكرة',
    'Next prayer': 'الصلاة التالية',
    'Adhan sound': 'صوت الأذان',
    'Coming soon': 'قريبًا',
    'Plays with the prayer alert, which lasts as long as the adhan': 'يُرفع مع تنبيه الصلاة، ويبقى التنبيه حتى ينتهي الأذان',
    'Adhan': 'الأذان',
    "Nova's adhan": 'أذان Nova',
    'Fajr adhan': 'أذان الفجر',
    "Nova's Fajr adhan": 'أذان الفجر من Nova',
    'Same as the adhan': 'مثل الأذان',
    'No file yet': 'لا يوجد ملف بعد',
    'Play': 'تشغيل',
    'Stop': 'إيقاف',
    'Choose file': 'اختيار ملف',
    'Default': 'الافتراضي',
    'Internet speed': 'سرعة الإنترنت',
    'Slow internet': 'بطء الإنترنت',
    'When the ping stays over 200 ms for a minute': 'عندما يزيد زمن الاستجابة على 200 ملّي ثانية لمدة دقيقة',
    'Now playing': 'قيد التشغيل',
    'Show media': 'إظهار الوسائط',
    'Title': 'العنوان',
    'New track card': 'بطاقة المقطع الجديد',
    'Open apps first': 'التطبيقات المفتوحة أولًا',
    'Apps with a window open come first in the Apps tab': 'تظهر التطبيقات التي لها نافذة مفتوحة أولًا في تبويب التطبيقات',
    'Tools': 'الأدوات',
    'Calculator': 'الآلة الحاسبة',
    // Themes
    'Give the island a new look. Every theme is free, and none of them makes Nova heavier; solid themes are even lighter.': 'امنح الجزيرة مظهرًا جديدًا. جميع المظاهر مجانية، ولا يزيد أيٌّ منها من استهلاك Nova، والمظاهر ذات اللون الثابت أخف.',
    'Theme': 'المظهر',
    'Lighter': 'أخف',
    'Original Nova': 'Nova الأصلي',
    'Liquid glass': 'زجاج سائل',
    'Bakka': 'بكّة',
    'Deep green and gold': 'أخضر داكن وذهبي',
    'Outline': 'الإطار',
    'Glass with a clean line': 'زجاج بخط واضح',
    'Midnight': 'منتصف الليل',
    'Pure black': 'أسود خالص',
    'Forest': 'الغابة',
    'Pine green': 'أخضر الصنوبر',
    'Snow': 'الثلج',
    'Winter glass': 'زجاج شتوي',
    'Cat': 'القطة',
    'Little ears, paws for eyes': 'أذنان صغيرتان، وعينان كالمخالب',
    'Galaxy': 'المجرة',
    'Violet sky and stars': 'سماء بنفسجية ونجوم',
    'Aurora': 'الشفق القطبي',
    'Northern lights': 'أضواء الشمال',
    'Ocean': 'المحيط',
    'Deep blue waves': 'أمواج زرقاء عميقة',
    'Nody': 'نودي',
    'Sakura pink, flowers for eyes': 'وردي الساكورا، وعينان كالزهور',
    'Lava': 'الحمم',
    'Glowing embers': 'جمر متوهج',
    'Retro': 'كلاسيكي',
    'Pixel green': 'أخضر البكسل',
    'Madar': 'مدار',
    'Graphite and mint': 'رمادي فحمي ولمسة نعناعية',
    // Companion
    'Nova': 'Nova',
    'Eyes': 'العينان',
    'At the adhan': 'عند الأذان',
    'Focus sessions': 'جلسات التركيز',
    'Celebrations': 'الاحتفالات',
    'Moments': 'لحظات',
    'Morning brief': 'موجز الصباح',
    'Welcome back': 'الترحيب بالعودة',
    'Break reminders': 'تذكير بالاستراحة',
    'Network': 'الشبكة',
    'Smart clipboard': 'الحافظة الذكية',
    'Links, colors, math': 'الروابط والألوان والعمليات الحسابية',
    'Water': 'الماء',
    'Water reminder': 'تذكير بشرب الماء',
    'Every': 'كل',
    'Daily goal': 'الهدف اليومي',
    'Less': 'أقل',
    'More': 'أكثر',
    // Prayer & Dhikr
    'Prayer alerts': 'تنبيهات الصلاة',
    'Hanafi Asr': 'العصر على المذهب الحنفي',
    'Hijri date': 'التاريخ الهجري',
    'Match local moon sighting': 'وفق رؤية الهلال المحلية',
    'One day less': 'يوم أقل',
    'One day more': 'يوم أكثر',
    'None': 'بلا تعديل',
    "Dhikr, du'a & Qur'an": 'الذكر والدعاء والقرآن',
    'Show reminders': 'إظهار التذكيرات',
    'On screen for': 'مدة الظهور',
    "Qur'an verses": 'آيات من القرآن',
    'Dhikr': 'الذكر',
    "Du'a": 'الدعاء',
    'Quiet hours': 'ساعات الهدوء',
    'From': 'من',
    'To': 'إلى',
    'Earlier': 'أبكر',
    'Later': 'أبعد',
    'Show one now': 'اعرض واحدًا الآن',
    // Alerts & Privacy
    'Alerts': 'التنبيهات',
    'Volume': 'مستوى الصوت',
    'Brightness': 'السطوع',
    'Battery': 'البطارية',
    'Keyboard language': 'لغة لوحة المفاتيح',
    'Caps Lock': 'Caps Lock',
    'News from Nova': 'أخبار Nova',
    'Plugins, updates and news from the Nova team. Nothing is sent from your PC.': 'إضافات وتحديثات وأخبار من فريق Nova. لا يُرسل أي شيء من جهازك.',
    'App notifications on the island': 'إشعارات التطبيقات على الجزيرة',
    "WhatsApp, Telegram and other apps. Turn off Windows' own banners to see each one once": 'واتساب وتيليجرام وغيرهما. أوقف لافتات Windows لترى كل إشعار مرة واحدة',
    'Screenshots': 'لقطات الشاشة',
    'Save screenshots to Shelf': 'حفظ لقطات الشاشة على الرف',
    'The file stays where it was saved': 'يبقى الملف حيث حُفظ',
    'Keep on Shelf': 'البقاء على الرف',
    'Privacy': 'الخصوصية',
    'Mic & camera dots': 'نقاط الميكروفون والكاميرا',
    'Clipboard history': 'سجل الحافظة',
    'Kept in memory only': 'يُحفظ في الذاكرة فقط',
    'Kept in memory only; clips you pin are saved on this PC': 'يُحفظ في الذاكرة فقط، وما تثبّته يُحفظ على هذا الجهاز',
    // Connections
    'Calendar': 'التقويم',
    'Upcoming meetings': 'الاجتماعات القادمة',
    'Save': 'حفظ',
    'Status': 'الحالة',
    'Not linked': 'غير مرتبط',
    'Linked · nothing in the next 24 h': 'مرتبط · لا شيء خلال 24 ساعة القادمة',
    'Use an https:// or webcal:// link': 'استخدم رابطًا يبدأ بـ https:// أو webcal://',
    'Connecting…': 'جارٍ الاتصال…',
    'How to get the link': 'كيف تحصل على الرابط',
    'Google:': 'Google:',
    'Settings › your calendar › Secret address in iCal format': 'الإعدادات › تقويمك › العنوان السري بتنسيق iCal',
    'Outlook:': 'Outlook:',
    'Settings › Calendar › Shared calendars › Publish (ICS)': 'الإعدادات › التقويم › التقويمات المشتركة › النشر (ICS)',
    'iCal link (https://… .ics)': 'رابط iCal (https://… .ics)',
    'Downloads': 'التنزيلات',
    'IDM Nova downloads': 'تنزيلات IDM Nova',
    'Browser downloads': 'تنزيلات المتصفح',
    'Chrome, Edge, Firefox and others': 'Chrome وEdge وFirefox وغيرها',
    'Download alerts': 'تنبيهات التنزيل',
    'Download folders': 'مجلدات التنزيل',
    'Where your browser saves files': 'حيث يحفظ متصفحك الملفات',
    'Windows Downloads': 'مجلد التنزيلات في Windows',
    'Remove folder': 'إزالة المجلد',
    'Add folder': 'إضافة مجلد',
    'Troubleshooting': 'استكشاف الأخطاء',
    'System helper': 'مساعد النظام',
    'Running': 'يعمل',
    'Starting…': 'جارٍ التشغيل…',
    'Stopped': 'متوقف',
    'Allow Nova Island in your antivirus, then restart it': 'اسمح لـ Nova Island في برنامج الحماية، ثم أعد تشغيله',
    'Error log': 'سجل الأخطاء',
    'Open': 'فتح',
    'Copy report': 'نسخ التقرير',
    'Report copied': 'نُسخ التقرير',
    'All settings': 'جميع الإعدادات',
    'Reset to defaults': 'استعادة الإعدادات الافتراضية',
    'Done': 'تم',
    'Show': 'إظهار',
    // Plugins
    'Extra features you add only if you want them. Every plugin is checked before it installs.': 'ميزات إضافية تضيفها إن شئت. تُفحص كل إضافة قبل تثبيتها.',
    'All': 'الكل',
    'Installed': 'المثبتة',
    'Loading…': 'جارٍ التحميل…',
    'Install from a file…': 'التثبيت من ملف…',
    'No plugins installed yet': 'لم تُثبَّت أي إضافة بعد',
    "Can't reach the plugin list. Check your connection.": 'تعذّر الوصول إلى قائمة الإضافات. تحقق من اتصالك.',
    'Installing…': 'جارٍ التثبيت…',
    'Install': 'تثبيت',
    'Remove': 'إزالة',
    'On': 'مفعّلة',
    'Install failed': 'تعذّر التثبيت',
    'Removed': 'أُزيلت',
    // the built-in Quran Radio plugin (its manifest is in English)
    'Quran Radio': 'إذاعة القرآن الكريم',
    "Live Qur'an radio on the island: Cairo, Saudi and 170+ reciters": 'إذاعة القرآن الكريم مباشرةً على الجزيرة: القاهرة والسعودية وأكثر من 170 قارئًا',
    "Qur'an radio: Cairo, Saudi and 140+ reciters, live or surah by surah": 'إذاعة القرآن الكريم: القاهرة والسعودية وأكثر من 140 قارئًا، بثًّا مباشرًا أو سورةً سورة',
    // the built-in Tasks plugin (1.8.3)
    'Tasks': 'المهام',
    'A to-do list on the island: due times, reminders and priority tags': 'قائمة مهام على الجزيرة: مواعيد وتذكيرات ووسوم للأولوية',
    'Station': 'المحطة',
    'Pause at prayer time': 'الإيقاف وقت الصلاة',
    'Stops the radio when the prayer comes in': 'تتوقف الإذاعة عند دخول وقت الصلاة',
    // Support
    'Nova Island is free, with no ads, and always will be. If it makes your day a little better:': 'Nova Island مجاني وبلا إعلانات، وسيبقى كذلك دائمًا. إن جعل يومك أجمل قليلًا:',
    'Buy us a coffee': 'اشترِ لنا قهوة',
    'Ko-fi · cards and PayPal': 'Ko-fi · البطاقات وPayPal',
    'From Egypt': 'من مصر',
    'Star on GitHub': 'نجمة على GitHub',
    'Free, and it helps others find Nova': 'مجانية، وتساعد الآخرين على اكتشاف Nova',
    '⭐ Star': '⭐ نجمة',
    // More by Nova
    'Other apps from the Nova team. Free and open source, like Nova Island.': 'تطبيقات أخرى من فريق Nova، مجانية ومفتوحة المصدر مثل Nova Island.',
    'Beautiful QR codes, in seconds. Links, PDFs, Wi-Fi, contacts and more.': 'رموز QR جميلة في ثوانٍ: روابط وملفات PDF وشبكات Wi-Fi وجهات اتصال والمزيد.',
    'Free': 'مجاني',
    'Open source': 'مفتوح المصدر',
    'Download': 'تنزيل',
    'More apps are on the way.': 'المزيد من التطبيقات قادم.',
  };
  // Arabic counts: one, two, 3 to 10, 11 and up
  const count = (n, [one, two, few, many]) => {
    n = Number(n);
    if (n === 1) return one;
    if (n === 2) return two;
    return `${n} ${n % 100 >= 3 && n % 100 <= 10 ? few : many}`;
  };
  // texts the page builds with numbers or names in them
  const PATTERNS = [
    [/^Place (\d+)$/, (m) => `المكان ${m[1]}`],
    [/^(\d+) glasses$/, (m) => count(m[1], ['كوب واحد', 'كوبان', 'أكواب', 'كوبًا'])],
    [/^(\d+) min$/, (m) => count(m[1], ['دقيقة واحدة', 'دقيقتان', 'دقائق', 'دقيقة'])],
    [/^(\d+) h$/, (m) => count(m[1], ['ساعة واحدة', 'ساعتان', 'ساعات', 'ساعة'])],
    [/^(\d+) s$/, (m) => count(m[1], ['ثانية واحدة', 'ثانيتان', 'ثوانٍ', 'ثانية'])],
    [/^⬇ ([\d,]+) installs · (.+)$/, (m) => `⬇ ${m[1]} تثبيت · ${m[2]}`],
    [/^([+−])(\d+) days?$/, (m) => `${m[1]}${m[2]} ${m[2] === '1' ? 'يوم' : 'أيام'}`],
    [/^Version (.+)$/, (m) => `الإصدار ${m[1]}`],
    [/^On now · battery (\d+)%$/, (m) => `مفعّل الآن · البطارية ${m[1]}%`],
    [/^Auto · (.+)$/, (m) => `تلقائي · ${m[1]}`],
    [/^Linked · next: (.+)$/, (m) => `مرتبط · التالي: ${m[1]}`],
    [/^Can't read it \((.+)\)$/, (m) => `تعذّرت قراءته (${m[1]})`],
    [/^(.+) theme$/, (m) => `مظهر ${AR[m[1]] || m[1]}`],
    [/^(.+) installed$/, (m) => `ثُبّتت ${m[1]}`],
    [/^⬇ ([\d,]+) installs$/, (m) => `⬇ ${m[1]} تثبيت`],
  ];
  const ATTRS = ['placeholder', 'title', 'aria-label'];
  const tidy = (t) => t.replace(/\s+/g, ' ').trim();
  const arabic = (en) => {
    const key = tidy(en);
    if (!key) return null;
    if (AR[key] != null) return AR[key];
    for (const [re, fn] of PATTERNS) {
      const m = key.match(re);
      if (m) return fn(m);
    }
    return null;
  };

  let lang = 'en';
  const done = new Set(); // Arabic we wrote ourselves, so the observer leaves it alone
  // a text node keeps its English, so English can come back
  function textNode(n) {
    const now = n.nodeValue;
    if (lang === 'ar') {
      if (done.has(now)) return;
      const ar = arabic(now);
      if (ar == null) return;
      n.__en = now;
      const lead = now.match(/^\s*/)[0];
      const trail = now.match(/\s*$/)[0];
      const value = lead + ar + trail;
      done.add(value);
      n.nodeValue = value;
    } else if (n.__en != null) {
      if (done.has(now)) n.nodeValue = n.__en;
      n.__en = null;
    }
  }
  function element(el) {
    for (const a of ATTRS) {
      const v = el.getAttribute(a);
      if (v == null) continue;
      const keep = `data-en-${a}`;
      if (lang === 'ar') {
        if (done.has(v)) continue;
        const ar = arabic(v);
        if (ar == null) continue;
        el.setAttribute(keep, v);
        done.add(ar);
        el.setAttribute(a, ar);
      } else if (el.hasAttribute(keep)) {
        el.setAttribute(a, el.getAttribute(keep));
        el.removeAttribute(keep);
      }
    }
  }
  function walk(root) {
    if (root.nodeType === Node.TEXT_NODE) return textNode(root);
    if (root.nodeType !== Node.ELEMENT_NODE) return;
    if (root.closest?.('[data-no-i18n], script, style')) return;
    element(root);
    const it = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => (n.parentElement?.closest('[data-no-i18n], script, style') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    for (let n = it.nextNode(); n; n = it.nextNode()) {
      if (n.nodeType === Node.TEXT_NODE) textNode(n);
      else element(n);
    }
  }

  function apply(next) {
    lang = next === 'ar' ? 'ar' : 'en';
    document.documentElement.lang = lang;
    document.documentElement.dir = lang === 'ar' ? 'rtl' : 'ltr';
    document.documentElement.classList.toggle('lang-ar', lang === 'ar');
    walk(document.body);
    const t = arabic(document.title.__en || document.title);
    if (lang === 'ar' && t) { document.title.__en = document.title; document.title = t; }
    for (const b of document.querySelectorAll('[data-lang]')) b.setAttribute('aria-checked', String(b.dataset.lang === lang));
    document.documentElement.classList.remove('i18n-wait'); // i18n-early.js kept it hidden till now
  }

  // what the page writes later
  new MutationObserver((list) => {
    if (lang !== 'ar') return;
    for (const m of list) {
      if (m.type === 'characterData') textNode(m.target);
      else if (m.type === 'attributes') element(m.target);
      else m.addedNodes.forEach(walk);
    }
  }).observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });

  for (const b of document.querySelectorAll('[data-lang]')) {
    b.addEventListener('click', () => {
      apply(b.dataset.lang);
      window.prefs.setFeatures({ prefsLang: b.dataset.lang });
    });
  }
  window.prefs.onFeatures((f) => { if (f && f.prefsLang && f.prefsLang !== lang) apply(f.prefsLang); });
  // (1.8.4) the app puts the language in the address (?lang=ar): applied now, before the first
  // paint, so Arabic never shows English first; the saved setting still has the last word
  if (new URLSearchParams(location.search).get('lang') === 'ar') apply('ar');
  window.prefs.features().then((f) => { if ((f?.prefsLang || 'en') !== lang) apply(f?.prefsLang); });
})();
