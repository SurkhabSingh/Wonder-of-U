from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import ctranslate2
from transformers import AutoTokenizer, NllbTokenizerFast
from transformers.utils import logging as transformers_logging


transformers_logging.set_verbosity_error()


SENTENCE_BREAK_PATTERN = re.compile(r"(?<=[.!?。！？])\s+")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--transcript-path", required=True)
    parser.add_argument("--output-path", required=True)
    parser.add_argument("--source-language", required=True)
    parser.add_argument("--target-language", required=True)
    return parser.parse_args()


def tokenize_length(tokenizer: AutoTokenizer, text: str) -> int:
    return len(tokenizer.encode(text, add_special_tokens=True))


def sentence_segments(paragraph: str) -> list[str]:
    collapsed = " ".join(part.strip() for part in paragraph.splitlines() if part.strip()).strip()
    if not collapsed:
        return []
    segments = [segment.strip() for segment in SENTENCE_BREAK_PATTERN.split(collapsed) if segment.strip()]
    return segments or [collapsed]


def split_oversized_segment(
    segment: str, tokenizer: AutoTokenizer, max_tokens: int
) -> list[str]:
    words = segment.split()
    if not words:
        return []

    chunks: list[str] = []
    current: list[str] = []
    for word in words:
        candidate = " ".join(current + [word]) if current else word
        if tokenize_length(tokenizer, candidate) <= max_tokens:
            current.append(word)
            continue

        if current:
            chunks.append(" ".join(current))
        current = [word]

    if current:
        chunks.append(" ".join(current))

    return chunks


def chunk_paragraph(paragraph: str, tokenizer: AutoTokenizer, max_tokens: int) -> list[str]:
    chunks: list[str] = []
    current: list[str] = []

    for segment in sentence_segments(paragraph):
        candidate = " ".join(current + [segment]) if current else segment
        if tokenize_length(tokenizer, candidate) <= max_tokens:
            current.append(segment)
            continue

        if current:
            chunks.append(" ".join(current))
            current = []

        if tokenize_length(tokenizer, segment) <= max_tokens:
            current = [segment]
            continue

        chunks.extend(split_oversized_segment(segment, tokenizer, max_tokens))

    if current:
        chunks.append(" ".join(current))

    return chunks


def chunk_text(text: str, tokenizer: AutoTokenizer, max_tokens: int) -> list[list[str]]:
    paragraphs = [paragraph.strip() for paragraph in re.split(r"\n\s*\n", text) if paragraph.strip()]
    return [chunk_paragraph(paragraph, tokenizer, max_tokens) for paragraph in paragraphs]


def load_tokenizer(model_dir: Path, source_language: str, target_language: str):
    if (model_dir / "tokenizer.json").is_file():
        try:
            return NllbTokenizerFast.from_pretrained(
                model_dir,
                src_lang=source_language,
                tgt_lang=target_language,
            )
        except Exception as error:
            if not (model_dir / "sentencepiece.bpe.model").is_file():
                raise RuntimeError(
                    "The translation tokenizer could not be loaded from tokenizer.json. "
                    f"Check the managed model files in {model_dir}: {error}"
                ) from error

    return AutoTokenizer.from_pretrained(
        model_dir,
        src_lang=source_language,
        tgt_lang=target_language,
        use_fast=False,
    )


def translate_chunk(
    translator: ctranslate2.Translator,
    tokenizer: AutoTokenizer,
    chunk: str,
    target_language: str,
) -> str:
    source_tokens = tokenizer.convert_ids_to_tokens(
        tokenizer.encode(chunk, add_special_tokens=True)
    )
    result = translator.translate_batch(
        [source_tokens],
        target_prefix=[[target_language]],
        max_batch_size=1,
    )[0]

    target_tokens = list(result.hypotheses[0])
    if target_tokens and target_tokens[0] == target_language:
        target_tokens = target_tokens[1:]

    target_ids = tokenizer.convert_tokens_to_ids(target_tokens)
    return tokenizer.decode(target_ids, skip_special_tokens=True).strip()


def run_translation(args: argparse.Namespace) -> str:
    transcript_text = Path(args.transcript_path).read_text(encoding="utf-8").strip()
    if not transcript_text:
        raise RuntimeError("The transcript file was empty, so translation could not run.")

    model_dir = Path(args.model_dir)
    tokenizer = load_tokenizer(model_dir, args.source_language, args.target_language)
    translator = ctranslate2.Translator(args.model_dir, device="cpu")

    model_max_length = getattr(tokenizer, "model_max_length", 512)
    if not isinstance(model_max_length, int) or model_max_length <= 0 or model_max_length > 2048:
        model_max_length = 512

    paragraph_groups = chunk_text(transcript_text, tokenizer, model_max_length)
    translated_paragraphs: list[str] = []
    for paragraph_chunks in paragraph_groups:
        translated_chunks = [
            translate_chunk(translator, tokenizer, chunk, args.target_language)
            for chunk in paragraph_chunks
        ]
        translated_paragraph = " ".join(chunk for chunk in translated_chunks if chunk).strip()
        if translated_paragraph:
            translated_paragraphs.append(translated_paragraph)

    output_text = "\n\n".join(translated_paragraphs).strip()
    if not output_text:
        raise RuntimeError("The translation output was empty.")

    return output_text


def main() -> int:
    args = parse_args()
    output_text = run_translation(args)

    output_path = Path(args.output_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(output_text + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SystemExit:
        raise
    except Exception as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
