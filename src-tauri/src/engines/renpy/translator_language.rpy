# Переключатель языка мода. Файл собирается в rpyc как один блок init python, поэтому внутри
# блока только ASCII и только API Python: операторы экранов компилятор мода не поддерживает.
# Переключатель показывается поверх главного меню и настроек, потому что разметка экранов
# у каждой игры своя. Выбор остаётся за игроком: config.language движок ставил бы при каждом запуске.
# Язык подставляет сборка мода (engines/renpy, `language_script`). Картинки это круги 40x40 в PNG: Frame растягивает их в капсулу,
# а встроены они здесь, чтобы мод добавлял в игру один файл. Список языков собирается из всех локализаций игры и модов:
# от трёх пунктов он сворачивается в стек. Последний пункт это язык скриптов: он есть всегда, но не обязательно английский.
# Смена языка держит игру секунду и больше, поэтому сначала едет ползунок, а язык меняется после него. Шрифт и акцентный цвет берутся у игры, чтобы переключатель выглядел её частью.
# Подписи латиницей: кириллицы в шрифте игры может не быть.
init python:
    import base64 as _translator_base64
    import time as _translator_time
    _TRANSLATOR_TRACK = "iVBORw0KGgoAAAANSUhEUgAAACgAAAAoCAYAAACM/rhtAAAA70lEQVR42u2ZsQ6DIBRFz9rZtf/Ah7g6s3Z1dmVm9XsbDSS20Talivc1fcmdVHLiQ7hcobyuQAvcgAEYnzSka226t0o1QAeEFaB3CunZ5giwC+ALoLbk05i7lAPijnBZMY39VfkDwNbeZlFL+wpwWf2nLa8Jt4SUaWtxu92JcFnu1byLAoBxaz56AbjNVjdCcFkPO04nCNgtAYMgYFi6klFUswtqhQEnttmzqQJObKtmU0UTmyxc1h/w9wHlPxL5ZUZ+oZbf6uTNggm7JW9Y5S2/iUOT/LHTxMHdRPRhIjySj9/MBJgmImATIfppvyHu7FPgexYbwDIAAAAASUVORK5CYII="
    _TRANSLATOR_THUMB = "iVBORw0KGgoAAAANSUhEUgAAACgAAAAoCAYAAACM/rhtAAAA+klEQVR42u2Zaw2DMBSF6wAHOMAJhnCAAxwgAQc4qAMc1MC3sJSFbMCyjse52ZqcX0DzpbeP04NziQ0ogApogR4IT+rjs/Gdwp3RgByoAb8A9E4+fpsfAZYBTQLUmsa+sr3gSmDYEW7S2Gf5LVxzANjLaKaWtDsBblL3UclPhntAKpU1rdxxQYSLVW7Nu0EAcFicjxeXdrvU8YQIYsrngLUgYD0H9IKAfu5KgqgKF+2QKmDlomdTBWzditlUUe+E4e76A/4EoPwikd9m5Ddq7aNO3ixYsVvahlXe8pu4NMlfO01c3E1EHybCI/n4zUyAaSICNhGiX/kb4gY4XunO4HpynwAAAABJRU5ErkJggg=="
    _translator_switch = {"entered": None, "from": 0, "to": None, "moved": 0.0, "open": False, "pending": False, "target": None, "items": None, "images": {}}
    _TRANSLATOR_CODES = {"russian": "RU", "english": "EN", "ukrainian": "UA", "spanish": "ES", "french": "FR", "german": "DE", "italian": "IT", "portuguese": "PT", "polish": "PL", "turkish": "TR", "chinese": "ZH", "japanese": "JA", "korean": "KO"}
    def _translator_label(name):
        code = _TRANSLATOR_CODES.get(name.lower())
        if code:
            return code
        return name.upper() if len(name) <= 3 else name[:10].capitalize()
    def _translator_items():
        state = _translator_switch
        if state["items"] is None:
            names = set(name for name in renpy.known_languages() if name)
            names.discard("__LANG__")
            items = [(_translator_label(name), name) for name in ["__LANG__"] + sorted(names)]
            state["items"] = items + [("Original", None)]
        return state["items"]
    def _translator_accent():
        try:
            return Color(gui.accent_color).rgb
        except Exception:
            return (1.0, 1.0, 1.0)
    def _translator_image(data, name, size):
        return im.Scale(im.Data(_translator_base64.b64decode(data), name), size, size)
    def _translator_images(height, inner, accent):
        images = _translator_switch["images"]
        key = (height, inner, accent)
        if key not in images:
            thumb = im.MatrixColor(_translator_image(_TRANSLATOR_THUMB, "translator_thumb.png", inner), im.matrix.tint(accent[0], accent[1], accent[2]))
            images[key] = (
                Frame(_translator_image(_TRANSLATOR_TRACK, "translator_track.png", height), height // 2, height // 2),
                Frame(thumb, inner // 2, inner // 2),
                Frame(im.Alpha(thumb, 0.14), inner // 2, inner // 2),
            )
        return images[key]
    def _translator_ease(progress):
        progress = min(max(progress, 0.0), 1.0) - 1.0
        return 1.0 + progress * progress * progress
    def _translator_enter(trans, st, at):
        progress = (_translator_time.time() - (_translator_switch["entered"] or 0.0)) / 0.3
        trans.alpha = min(max(progress, 0.0), 1.0)
        trans.yoffset = int((_translator_ease(progress) - 1.0) * config.screen_height * 0.02)
        return 0 if progress < 1.0 else None
    def _translator_thumb_mover(step, vertical):
        def move(trans, st, at):
            state = _translator_switch
            progress = (_translator_time.time() - state["moved"]) / 0.18
            start, end = state["from"], state["to"]
            offset = (start + (end - start) * _translator_ease(progress)) * step
            if vertical:
                trans.yoffset = offset
            else:
                trans.xoffset = offset
            trans.subpixel = True
            return 0 if progress < 1.0 else None
        return move
    def _translator_toggle():
        _translator_switch["open"] = not _translator_switch["open"]
    def _translator_pick(language):
        state = _translator_switch
        if state["pending"]:
            return
        if language == _preferences.language:
            state["open"] = False
            return
        state["pending"] = True
        state["target"] = language
    def _translator_apply():
        state = _translator_switch
        state["pending"] = False
        state["open"] = False
        renpy.change_language(state["target"])
    def _translator_language_screen(**kwargs):
        state = _translator_switch
        items = _translator_items()
        vertical = len(items) > 2
        collapsed = vertical and not state["open"]
        height = max(30, int(config.screen_height * 0.05))
        pad = max(3, height // 10)
        segment = int(height * 2.5)
        inner = height - 2 * pad
        margin = height // 3
        now = _translator_time.time()
        if state["entered"] is None:
            state["entered"] = now
        language = state["target"] if state["pending"] else _preferences.language
        selected = None
        for index, item in enumerate(items):
            if item[1] == language:
                selected = index
        if selected is not None and selected != state["to"]:
            if state["to"] is None:
                state["from"] = selected
            else:
                state["from"] = state["to"]
                state["moved"] = now
            state["to"] = selected
        accent = _translator_accent()
        track, thumb, hover = _translator_images(height, inner, tuple(accent))
        active = "#111111" if 0.299 * accent[0] + 0.587 * accent[1] + 0.114 * accent[2] > 0.6 else "#ffffff"
        look = dict(
            style="default",
            text_style="button_text",
            xsize=segment,
            ysize=inner,
            keyboard_focus=False,
            idle_background=None,
            hover_background=hover,
            selected_idle_background=None,
            selected_hover_background=None,
            text_size=int(height * 0.42),
            text_outlines=[],
            text_bold=False,
            text_italic=False,
            text_xalign=0.5,
            text_yalign=0.5,
            text_idle_color="#ffffffc0",
            text_hover_color="#ffffff",
            text_selected_idle_color=active,
            text_selected_hover_color=active,
        )
        if collapsed:
            width, total = segment + 2 * pad, height
        elif vertical:
            width, total = segment + 2 * pad, len(items) * inner + 2 * pad
        else:
            width, total = len(items) * segment + 2 * pad, height
        ui.at(Transform(function=_translator_enter))
        ui.fixed(xanchor=1.0, xpos=config.screen_width - margin, ypos=margin, xsize=width, ysize=total)
        ui.add(track)
        if collapsed:
            label = items[selected][0] if selected is not None else "Language"
            ui.textbutton(label + u" {font=DejaVuSans.ttf}\u25be{/font}", action=Function(_translator_toggle), xpos=pad, ypos=pad, **look)
        else:
            step = inner if vertical else segment
            if selected is not None:
                ui.at(Transform(function=_translator_thumb_mover(step, vertical)))
                ui.fixed(xpos=pad, ypos=pad, xsize=segment, ysize=inner)
                ui.add(thumb)
                ui.close()
            for index, item in enumerate(items):
                ui.textbutton(
                    item[0],
                    action=Function(_translator_pick, item[1]),
                    selected=index == selected,
                    xpos=pad + (0 if vertical else index * segment),
                    ypos=pad + (index * inner if vertical else 0),
                    **look
                )
        if state["pending"]:
            ui.timer(0.18, action=Function(_translator_apply))
        ui.close()
    renpy.define_screen("translator_language", _translator_language_screen, zorder="1000")
    def _translator_language_sync():
        wanted = any(renpy.get_screen(name) for name in ("main_menu", "preferences"))
        shown = renpy.get_screen("translator_language") is not None
        if wanted and not shown:
            renpy.show_screen("translator_language")
        elif shown and not wanted:
            renpy.hide_screen("translator_language")
    config.start_interact_callbacks.append(_translator_language_sync)
    config.default_language = "__LANG__"
    if not persistent._translator_language_applied:
        persistent._translator_language_applied = True
        _preferences.language = "__LANG__"
