#!/usr/bin/env python3
"""Deterministic tests for the bench scoring itself (no binary, no models).

    python3 bench/tests.py

Runs in seconds on any machine and gates the scoring semantics in CI:
tokenization, WER/CER sensitivity (inventions, omissions, order, empty
output), speaker-error accounting (empty output, minority speaker, short
replies) and the threshold checker. The full bench (run.py) still needs the
built binary and the models; this file needs only the standard library.
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run import check, edit_distance, score_text, score_turns, wer_cer, words


def md(*lines):
    """Transcript markdown from (time, label, text) tuples, time as m:ss."""
    return "".join(f"**[{t}] {label}:** {text}\n\n" for t, label, text in lines)


def truth_line(speaker, side, start, end, text):
    return {"speaker": speaker, "side": side, "start": start, "end": end, "text": text}


class Tokenization(unittest.TestCase):
    def test_french_accents_survive(self):
        self.assertEqual(words("Café, d'accord !"), ["café", "d'accord"])
        self.assertEqual(words("Oui."), ["oui"])

    def test_case_and_punctuation_ignored(self):
        self.assertEqual(words("Hello,  WORLD"), ["hello", "world"])


class ErrorRates(unittest.TestCase):
    TRUTH = [
        truth_line("You", "mic", 0, 3, "the cat sat on the mat"),
        truth_line("Anna", "computer", 4, 7, "and the dog barked loudly"),
    ]

    def test_perfect_transcript_scores_zero(self):
        m = md(("00:00", "You", "the cat sat on the mat"),
               ("00:04", "Anna", "and the dog barked loudly"))
        self.assertEqual(wer_cer(m, self.TRUTH), {"wer": 0.0, "cer": 0.0})

    def test_inventions_hurt(self):
        m = md(("00:00", "You", "the cat sat on the mat with a purple banana"),
               ("00:04", "Anna", "and the dog barked loudly"))
        self.assertGreater(wer_cer(m, self.TRUTH)["wer"], 0.0)

    def test_omissions_hurt(self):
        m = md(("00:00", "You", "the cat sat"),
               ("00:04", "Anna", "and the dog barked loudly"))
        r = wer_cer(m, self.TRUTH)
        self.assertGreater(r["wer"], 0.0)
        # Recall alone would still show a high "found"; WER must not.
        self.assertLess(score_text(m, self.TRUTH)["found"], 1.0)

    def test_wrong_order_hurts(self):
        # Same words, each said at the other's moment: recall is blind
        # (every word is near its truth), WER is not.
        m = md(("00:00", "You", "and the dog barked loudly"),
               ("00:04", "Remote", "the cat sat on the mat"))
        self.assertEqual(score_text(m, self.TRUTH)["found"], 1.0)
        self.assertGreater(wer_cer(m, self.TRUTH)["wer"], 0.0)

    def test_empty_output_scores_worst(self):
        r = wer_cer("", self.TRUTH)
        self.assertEqual((r["wer"], r["cer"]), (1.0, 1.0))
        s = score_text("", self.TRUTH)
        self.assertEqual((s["found"], s["side"], s["person"]), (0.0, 0.0, 0.0))

    def test_edit_distance_basics(self):
        self.assertEqual(edit_distance([], []), 0)
        self.assertEqual(edit_distance(["a"], ["a"]), 0)
        self.assertEqual(edit_distance(["a"], ["b"]), 1)
        self.assertEqual(edit_distance(["a", "b"], ["a"]), 1)


class SpeakerError(unittest.TestCase):
    TRUTH = [
        {"speaker": "A", "start": 0.0, "end": 60.0},
        {"speaker": "B", "start": 60.0, "end": 90.0},  # minority speaker
    ]

    def test_empty_turns_is_total_error(self):
        r = score_turns([], self.TRUTH)
        self.assertEqual(r["speaker error"], 1.0)
        self.assertEqual(r["speakers"], "0/2")

    def test_perfect_turns_score_zero(self):
        turns = [
            {"speaker": 0, "start": 0.0, "end": 60.0},
            {"speaker": 1, "start": 60.0, "end": 90.0},
        ]
        self.assertEqual(score_turns(turns, self.TRUTH)["speaker error"], 0.0)

    def test_merged_minority_speaker_hurts(self):
        # B merged into A: a third of the speech is misattributed.
        turns = [{"speaker": 0, "start": 0.0, "end": 90.0}]
        r = score_turns(turns, self.TRUTH)
        self.assertGreater(r["speaker error"], 0.2)
        self.assertEqual(r["speakers"], "1/2")

    def test_missing_coverage_hurts(self):
        # Only the first half covered: the missing half must not score 0.
        turns = [{"speaker": 0, "start": 0.0, "end": 45.0}]
        self.assertGreater(score_turns(turns, self.TRUTH)["speaker error"], 0.3)


class ShortReplies(unittest.TestCase):
    # Labels the bench understands: You (mic) and Remote (computer).
    TRUTH = [
        truth_line("Remote", "computer", 0, 1, "Oui."),
        truth_line("You", "mic", 1, 2, "Oui."),
        truth_line("Remote", "computer", 3, 4, "D'accord."),
        truth_line("You", "mic", 4, 5, "D'accord."),
    ]

    def test_short_replies_count_when_kept(self):
        m = md(("00:00", "Remote", "Oui."),
               ("00:01", "You", "Oui."),
               ("00:03", "Remote", "D'accord."),
               ("00:04", "You", "D'accord."))
        s = score_text(m, self.TRUTH)
        self.assertEqual(s["found"], 1.0)
        self.assertEqual(s["side"], 1.0)

    def test_dropped_short_replies_hurt(self):
        # The app deleted the mic's "Oui." as echo: the side recall and the
        # WER must show it (plain "found" cannot: the same word survives
        # nearby on the other line).
        m = md(("00:00", "Remote", "Oui."),
               ("00:03", "Remote", "D'accord."),
               ("00:04", "You", "D'accord."))
        s = score_text(m, self.TRUTH)
        self.assertLess(s["side"], 1.0)
        self.assertGreater(wer_cer(m, self.TRUTH)["wer"], 0.0)


class Thresholds(unittest.TestCase):
    def test_errors_fail_even_without_thresholds(self):
        self.assertEqual(check({"ami": {"error": "process failed"}}, {}),
                         ["ami: process failed"])

    def test_missing_measurements_do_not_pass(self):
        limits = {"min": {"found": 0}, "max": {"lines": 0}, "all_speakers": True}
        failures = check({"case": {}}, {"case": limits})
        self.assertEqual(len(failures), 3)
        self.assertTrue(all("missing" in failure for failure in failures))

    def test_checker_catches_low_recall_and_high_wer(self):
        thresholds = {"case": {"min": {"found": 0.9, "person": 0.9},
                               "max": {"leaked": 0, "wer": 0.2}}}
        ok = {"found": 0.95, "person": 0.95, "leaked": 0, "wer": 0.1}
        self.assertEqual(check({"case": ok}, thresholds), [])
        bad = {"found": 0.5, "person": 0.95, "leaked": 0, "wer": 0.1}
        self.assertTrue(any("found" in f for f in check({"case": bad}, thresholds)))
        bad_wer = {"found": 0.95, "person": 0.95, "leaked": 0, "wer": 0.5}
        self.assertTrue(any("wer" in f for f in check({"case": bad_wer}, thresholds)))

    def test_checker_requires_every_voice(self):
        thresholds = {"case": {"all_speakers": True}}
        self.assertEqual(check({"case": {"speakers": "2/2"}}, thresholds), [])
        self.assertTrue(check({"case": {"speakers": "1/2"}}, thresholds))


if __name__ == "__main__":
    unittest.main(verbosity=2)
