import importlib.util
from pathlib import Path
import tempfile
import unittest

from word_attribution import attribute, read_words, score_words


def word(speaker='a', start=0, end=1, text='hello'):
    return dict(session_id='demo', speaker=speaker, start_time=start, end_time=end, words=text)


class AttributionTests(unittest.TestCase):
    def test_unique_identity_across_fragments_and_conservative_abstention(self):
        reference = [word()]
        def segment(start, end, speaker):
            return dict(start=start, end=end, speaker=speaker)
        a = [segment(0, .4, 'one'), segment(.5, 1, 'one')]
        self.assertEqual(attribute(reference, a, 2)[0][0]['speaker'], 'one')
        for extra, reason in [([segment(.9, 1, 'two')], 'multiple_speakers'),
                              ([segment(.9, 1, 'UNKNOWN')], 'unknown_activity')]:
            hyp, abstained = attribute(reference, a + extra, 2)
            self.assertEqual(hyp, [])
            self.assertEqual(abstained, {reason: 1})
        self.assertEqual(attribute(reference, [], 2), ([], {'no_activity': 1}))
        with self.assertRaises(ValueError):
            attribute(reference, [segment(0, float('nan'), 'one')], 2)

    def test_xml_normalization_and_exclusions(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'words.xml'
            path.write_text('<root><w starttime="0" endtime="1">Um, 2.5!</w>'
                            '<w starttime="1" endtime="1">bad</w><w>missing</w>'
                            '<w starttime="1" endtime="2">...</w><vocalsound>noise</vocalsound></root>')
            data, excluded = read_words(path, 'demo', 'a', 3)
        self.assertEqual([w['words'] for w in data], ['um', '2.5'])
        self.assertEqual(sum(excluded.values()), 3)


@unittest.skipUnless(importlib.util.find_spec('meeteval'), 'optional pinned MeetEval environment')
class MeetEvalConformanceTests(unittest.TestCase):
    def test_global_permutation_and_all_unknown_penalty(self):
        ref = [word(), word('b', 2, 3, 'bye')]
        hyp = [dict(ref[0], speaker='two'), dict(ref[1], speaker='one')]
        for score in score_words(ref, hyp, .5).values():
            self.assertEqual(score['errors'], 0)
        for score in score_words(ref, [], .5).values():
            self.assertEqual(score['error_rate'], 1)
            self.assertEqual(score['deletions'], 2)

    def test_tcpwer_rejects_distant_matching_words(self):
        ref = [word()]
        hyp = [word('one', 10, 11)]
        scores = score_words(ref, hyp, .5)
        self.assertEqual(scores['cpwer']['errors'], 0)
        self.assertGreater(scores['tcpwer']['errors'], 0)


if __name__ == '__main__':
    unittest.main()
