//! Команды глоссария: разбор говорящих, персонажи, термины, корзина названий, токены.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};

use super::{
    Character, CharacterId, Form, GlossaryOverview, GlossaryService, NameKey, NamesService, SpeakerEntry, SpeakerSuggestion, Term,
    TermCategory, TermId, TermSuggestion, Token, TokenSuggestion,
};
use crate::common::{AppError, Gender, Timestamp};
use crate::engines::SpeakerKey;
use crate::llm::{ProfileId, RunProgress};
use crate::projects::ProjectId;
use crate::state::{AppState, blocking};
use crate::translation::commands::RowRef;

/// Как разобран говорящий.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerState {
    /// В «Новых»: ещё не разобран.
    New,
    /// Связан с персонажами.
    Character,
    /// Запись говорящего без персонажей.
    Speaker,
    Discarded,
}

/// Говорящий текущего снимка.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerDto {
    pub key: String,
    pub label: Option<String>,
    pub lines: u32,
    pub state: SpeakerState,
    /// Связанные персонажи, основной код первым.
    pub characters: Vec<RowRef>,
    /// Есть запись говорящего: свой перевод подписи и род.
    pub has_entry: bool,
    pub translation: Option<String>,
    pub gender: Option<Gender>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FormDto {
    pub text: String,
    pub translation: Option<String>,
}

impl From<&Form> for FormDto {
    fn from(form: &Form) -> Self {
        Self { text: form.text().to_string(), translation: form.translation().map(str::to_string) }
    }
}

fn forms(forms: &[FormDto]) -> Result<Vec<Form>, AppError> {
    Ok(forms
        .iter()
        .filter(|form| !form.text.trim().is_empty())
        .map(|form| Form::new(&form.text, form.translation.as_deref()))
        .collect::<Result<Vec<_>, _>>()?)
}

/// Персонаж. Окно присылает его целиком.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterDto {
    pub id: RowRef,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub translated_first_name: Option<String>,
    pub translated_last_name: Option<String>,
    pub gender: Option<Gender>,
    pub speech: Option<String>,
    pub description: Option<String>,
    pub forms: Vec<FormDto>,
}

impl From<&Character> for CharacterDto {
    fn from(character: &Character) -> Self {
        let text = |value: Option<&str>| value.map(str::to_string);
        Self {
            id: RowRef(character.id().map_or(0, CharacterId::get)),
            first_name: text(character.first_name()),
            last_name: text(character.last_name()),
            translated_first_name: text(character.translated_first_name()),
            translated_last_name: text(character.translated_last_name()),
            gender: character.gender(),
            speech: text(character.speech()),
            description: text(character.description()),
            forms: character.forms().iter().map(FormDto::from).collect(),
        }
    }
}

impl CharacterDto {
    /// Форма, совпавшая с именем или другой формой, это ошибка: у одного написания был бы второй перевод.
    fn into_character(self) -> Result<Character, AppError> {
        let text = |value: Option<String>| value.as_deref().and_then(crate::common::non_empty);
        let mut character = Character::restore(
            CharacterId::from_row(self.id.0),
            text(self.first_name),
            text(self.last_name),
            text(self.translated_first_name),
            text(self.translated_last_name),
            self.gender,
            text(self.speech),
            text(self.description),
            Vec::new(),
        );
        for form in forms(&self.forms)? {
            character.add_form(form)?;
        }
        Ok(character)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TermDto {
    pub id: RowRef,
    pub name: String,
    pub translation: Option<String>,
    pub category: TermCategory,
    /// Предположение модели: идёт в запросы с пометкой, пока его не подтвердят.
    pub guess: bool,
    pub description: Option<String>,
    pub forms: Vec<FormDto>,
}

impl From<&Term> for TermDto {
    fn from(term: &Term) -> Self {
        Self {
            id: RowRef(term.id().map_or(0, TermId::get)),
            name: term.name().to_string(),
            translation: term.translation().map(str::to_string),
            category: term.category(),
            guess: term.is_guess(),
            description: term.description().map(str::to_string),
            forms: term.forms().iter().map(FormDto::from).collect(),
        }
    }
}

impl TermDto {
    fn into_term(self) -> Result<Term, AppError> {
        let text = |value: Option<String>| value.as_deref().and_then(crate::common::non_empty);
        let mut term = Term::restore(
            TermId::from_row(self.id.0),
            &self.name,
            text(self.translation),
            self.category,
            self.guess,
            text(self.description),
            Vec::new(),
        )?;
        for form in forms(&self.forms)? {
            term.add_form(form)?;
        }
        Ok(term)
    }
}

/// Название в корзине.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscardedTermDto {
    pub key: String,
    pub text: String,
    /// Передано персонажу, а не мусор.
    pub given_to_character: bool,
}

/// Токен из текста игры или глоссария.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenDto {
    /// Без скобок.
    pub name: String,
    pub value: Option<String>,
    pub note: Option<String>,
    /// Реплик текущего снимка с этим токеном.
    pub lines: u32,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryDto {
    pub speakers: Vec<SpeakerDto>,
    pub characters: Vec<CharacterDto>,
    pub terms: Vec<TermDto>,
    pub discarded_terms: Vec<DiscardedTermDto>,
    pub tokens: Vec<TokenDto>,
}

impl From<GlossaryOverview> for GlossaryDto {
    fn from(overview: GlossaryOverview) -> Self {
        let GlossaryOverview { data, speakers, discarded_speakers, discarded_terms, game_tokens, .. } = overview;
        let discarded: Vec<&SpeakerKey> = discarded_speakers.iter().map(|speaker| &speaker.key).collect();
        let speakers = speakers
            .into_iter()
            .map(|item| {
                let key = &item.speaker.key;
                let mut links: Vec<_> = data.links.iter().filter(|link| &link.speaker == key).collect();
                links.sort_by_key(|link| !link.is_primary);
                let entry = data.entries.iter().find(|entry| entry.key() == key);
                let state = if !links.is_empty() {
                    SpeakerState::Character
                } else if entry.is_some() {
                    SpeakerState::Speaker
                } else if discarded.contains(&key) {
                    SpeakerState::Discarded
                } else {
                    SpeakerState::New
                };
                SpeakerDto {
                    key: key.as_str().to_string(),
                    label: item.speaker.label.clone(),
                    lines: item.lines as u32,
                    state,
                    characters: links.iter().map(|link| RowRef(link.character.get())).collect(),
                    has_entry: entry.is_some(),
                    translation: entry.and_then(SpeakerEntry::translation).map(str::to_string),
                    gender: entry.and_then(SpeakerEntry::gender),
                }
            })
            .collect();

        let mut tokens: BTreeMap<String, TokenDto> = game_tokens
            .into_iter()
            .map(|(name, lines)| (name.clone(), TokenDto { name, value: None, note: None, lines: lines as u32 }))
            .collect();
        for token in &data.tokens {
            let item = tokens
                .entry(token.name().to_string())
                .or_insert_with(|| TokenDto { name: token.name().to_string(), value: None, note: None, lines: 0 });
            item.value = token.value().map(str::to_string);
            item.note = token.note().map(str::to_string);
        }

        Self {
            speakers,
            characters: data.characters.iter().map(CharacterDto::from).collect(),
            terms: data.terms.iter().map(TermDto::from).collect(),
            discarded_terms: discarded_terms
                .into_iter()
                .map(|term| DiscardedTermDto {
                    key: term.key.as_str().to_string(),
                    text: term.text,
                    given_to_character: term.reason == super::DiscardReason::GivenToCharacter,
                })
                .collect(),
            tokens: tokens.into_values().collect(),
        }
    }
}

fn project(id: &str) -> Result<ProjectId, AppError> {
    Ok(ProjectId::parse(id)?)
}

fn speaker(key: &str) -> Result<SpeakerKey, AppError> {
    Ok(SpeakerKey::new(key)?)
}

/// Глоссарий проекта целиком.
#[tauri::command]
#[specta::specta]
pub async fn glossary(app: AppHandle, project: String) -> Result<GlossaryDto, String> {
    blocking(app, move |state| Ok(GlossaryService { state }.overview(self::project(&project)?)?.into())).await
}

/// «Персонаж» в «Новых говорящих». Возвращает id нового персонажа.
#[tauri::command]
#[specta::specta]
pub async fn speaker_to_character(app: AppHandle, project: String, speaker: String) -> Result<RowRef, String> {
    blocking(app, move |state| {
        let id = GlossaryService { state }.speaker_to_character(self::project(&project)?, &self::speaker(&speaker)?)?;
        Ok(RowRef(id.get()))
    })
    .await
}

/// «Взять всех» в «Новых говорящих». Возвращает число новых персонажей.
#[tauri::command]
#[specta::specta]
pub async fn all_speakers_to_characters(app: AppHandle, project: String) -> Result<u32, String> {
    blocking(app, move |state| Ok(GlossaryService { state }.all_speakers_to_characters(self::project(&project)?)?.len() as u32)).await
}

/// Связывает говорящего с персонажами.
#[tauri::command]
#[specta::specta]
pub async fn link_speaker(app: AppHandle, project: String, speaker: String, characters: Vec<RowRef>) -> Result<(), String> {
    blocking(app, move |state| {
        let characters: Vec<CharacterId> = characters.iter().map(|id| CharacterId::from_row(id.0)).collect();
        GlossaryService { state }.link_speaker(self::project(&project)?, &self::speaker(&speaker)?, &characters)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn unlink_speaker(app: AppHandle, project: String, speaker: String, character: RowRef) -> Result<(), String> {
    blocking(app, move |state| {
        GlossaryService { state }.unlink_speaker(self::project(&project)?, &self::speaker(&speaker)?, CharacterId::from_row(character.0))
    })
    .await
}

/// «Оставить говорящим».
#[tauri::command]
#[specta::specta]
pub async fn keep_as_speaker(app: AppHandle, project: String, speaker: String) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.keep_as_speaker(self::project(&project)?, &self::speaker(&speaker)?)).await
}

/// Перевод подписи и род записи говорящего.
#[tauri::command]
#[specta::specta]
pub async fn save_speaker_entry(
    app: AppHandle,
    project: String,
    speaker: String,
    translation: String,
    gender: Option<Gender>,
) -> Result<(), String> {
    blocking(app, move |state| {
        let mut entry = SpeakerEntry::new(self::speaker(&speaker)?, Timestamp::now());
        entry.set_translation(&translation);
        entry.set_gender(gender);
        GlossaryService { state }.save_speaker_entry(self::project(&project)?, &entry)
    })
    .await
}

/// Убирает запись говорящего.
#[tauri::command]
#[specta::specta]
pub async fn delete_speaker_entry(app: AppHandle, project: String, speaker: String) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.delete_speaker_entry(self::project(&project)?, &self::speaker(&speaker)?)).await
}

#[tauri::command]
#[specta::specta]
pub async fn discard_speaker(app: AppHandle, project: String, speaker: String) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.discard_speaker(self::project(&project)?, &self::speaker(&speaker)?)).await
}

#[tauri::command]
#[specta::specta]
pub async fn restore_speaker(app: AppHandle, project: String, speaker: String) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.restore_speaker(self::project(&project)?, &self::speaker(&speaker)?)).await
}

/// Персонаж, которого только упоминают. Возвращает его id.
#[tauri::command]
#[specta::specta]
pub async fn create_character(app: AppHandle, project: String, first_name: String) -> Result<RowRef, String> {
    blocking(app, move |state| Ok(RowRef(GlossaryService { state }.create_character(self::project(&project)?, &first_name)?.get()))).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_character(app: AppHandle, project: String, character: CharacterDto) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.save_character(self::project(&project)?, &character.into_character()?)).await
}

/// Удаляет персонажа. Его говорящие без других связей уходят в корзину.
#[tauri::command]
#[specta::specta]
pub async fn delete_character(app: AppHandle, project: String, character: RowRef) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.delete_character(self::project(&project)?, CharacterId::from_row(character.0))).await
}

/// Новый термин. Возвращает его id.
#[tauri::command]
#[specta::specta]
pub async fn take_term(app: AppHandle, project: String, name: String, category: TermCategory) -> Result<RowRef, String> {
    blocking(app, move |state| Ok(RowRef(GlossaryService { state }.take_term(self::project(&project)?, &name, category)?.get()))).await
}

/// Сохраняет термин целиком. Подтверждение предположения это `guess: false`.
#[tauri::command]
#[specta::specta]
pub async fn save_term(app: AppHandle, project: String, term: TermDto) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.save_term(self::project(&project)?, &term.into_term()?)).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_term(app: AppHandle, project: String, term: RowRef) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.delete_term(self::project(&project)?, TermId::from_row(term.0))).await
}

/// Термин становится формой персонажа, название уходит в корзину.
#[tauri::command]
#[specta::specta]
pub async fn give_term_to_character(app: AppHandle, project: String, term: RowRef, character: RowRef) -> Result<(), String> {
    blocking(app, move |state| {
        GlossaryService { state }.give_term_to_character(
            self::project(&project)?,
            TermId::from_row(term.0),
            CharacterId::from_row(character.0),
        )
    })
    .await
}

/// Название в корзину: модель не предложит его снова.
#[tauri::command]
#[specta::specta]
pub async fn discard_term_name(app: AppHandle, project: String, name: String) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.discard_term_name(self::project(&project)?, &name)).await
}

#[tauri::command]
#[specta::specta]
pub async fn restore_term_name(app: AppHandle, project: String, key: String) -> Result<(), String> {
    blocking(app, move |state| GlossaryService { state }.restore_term_name(self::project(&project)?, &NameKey::from_stored(key))).await
}

/// Значение или пояснение токена. Пустые оба удаляют запись.
#[tauri::command]
#[specta::specta]
pub async fn save_token(app: AppHandle, project: String, name: String, value: String, note: String) -> Result<(), String> {
    blocking(app, move |state| {
        let mut token = Token::new(&name)?;
        token.set_value(&value);
        token.set_note(&note);
        GlossaryService { state }.save_token(self::project(&project)?, &token)
    })
    .await
}

/// Предложение перевода термина.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TermSuggestionDto {
    pub id: RowRef,
    pub translation: String,
    /// `None`: категория остаётся прежней.
    pub category: Option<TermCategory>,
    /// Только у термина без своего описания.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSuggestionDto {
    pub key: String,
    pub translation: String,
    pub gender: Option<Gender>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenSuggestionDto {
    pub name: String,
    pub value: Option<String>,
    pub note: Option<String>,
}

/// Предложения запуска. `error`: запуск оборвался, готовые предложения всё равно пришли.
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TermSuggestionsDto {
    pub items: Vec<TermSuggestionDto>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSuggestionsDto {
    pub items: Vec<SpeakerSuggestionDto>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TokenSuggestionsDto {
    pub items: Vec<TokenSuggestionDto>,
    pub error: Option<String>,
}

fn profile(id: Option<String>) -> Result<Option<ProfileId>, AppError> {
    Ok(id.as_deref().map(ProfileId::parse).transpose()?)
}

/// Перевод терминов моделью. В базу ничего не пишется: отмеченное в отборе окно записывает через `save_term`.
#[tauri::command]
#[specta::specta]
pub async fn suggest_terms(
    state: State<'_, AppState>,
    project: String,
    terms: Vec<RowRef>,
    profile: Option<String>,
    progress: Channel<RunProgress>,
) -> Result<TermSuggestionsDto, String> {
    let terms = terms.iter().map(|id| TermId::from_row(id.0)).collect();
    let found = NamesService { state: &state }.translate_terms(self::project(&project)?, terms, self::profile(profile)?, progress).await?;
    Ok(TermSuggestionsDto {
        items: found
            .items
            .into_iter()
            .map(|item: TermSuggestion| TermSuggestionDto {
                id: RowRef(item.id.get()),
                translation: item.translation,
                category: item.category,
                description: item.description,
            })
            .collect(),
        error: found.error,
    })
}

/// Перевод подписей и род говорящих моделью. Отмеченное окно записывает через `save_speaker_entry`.
#[tauri::command]
#[specta::specta]
pub async fn suggest_speakers(
    state: State<'_, AppState>,
    project: String,
    speakers: Vec<String>,
    profile: Option<String>,
    progress: Channel<RunProgress>,
) -> Result<SpeakerSuggestionsDto, String> {
    let speakers = speakers.iter().map(|key| speaker(key)).collect::<Result<Vec<_>, _>>()?;
    let found = NamesService { state: &state }.suggest_speakers(self::project(&project)?, speakers, self::profile(profile)?, progress).await?;
    Ok(SpeakerSuggestionsDto {
        items: found
            .items
            .into_iter()
            .map(|item: SpeakerSuggestion| SpeakerSuggestionDto {
                key: item.key.as_str().to_string(),
                translation: item.translation,
                gender: item.gender,
            })
            .collect(),
        error: found.error,
    })
}

/// Значения и пояснения токенов моделью. Отмеченное окно записывает через `save_token`.
#[tauri::command]
#[specta::specta]
pub async fn suggest_tokens(
    state: State<'_, AppState>,
    project: String,
    tokens: Vec<String>,
    profile: Option<String>,
    progress: Channel<RunProgress>,
) -> Result<TokenSuggestionsDto, String> {
    let found = NamesService { state: &state }.suggest_tokens(self::project(&project)?, tokens, self::profile(profile)?, progress).await?;
    Ok(TokenSuggestionsDto {
        items: found
            .items
            .into_iter()
            .map(|item: TokenSuggestion| TokenSuggestionDto { name: item.name, value: item.value, note: item.note })
            .collect(),
        error: found.error,
    })
}
