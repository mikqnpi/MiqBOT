# MiqBOT

MiqBOT のシンプルな雛形です。  
このリポジトリは GitHub でそのまま運用できる最小構成（Python パッケージ＋CI）として作成されています。

## 特徴

- 最小限のボット本体クラス（`MiqBot`）
- そのまま起動できる CLI エントリポイント
- GitHub Actions による CI（インポート＋簡易実行）
- 将来の実装（Discord / Slack / API bot など）をすぐ追加できる構成

## 使い方

```bash
python -m venv .venv
.venv\Scripts\Activate.ps1  # Windows
# .venv/bin/activate        # macOS / Linux
pip install -e .
python -m miqbot.main --name MiqBOT --message "Hello, world"
```

## 発展

- 必要なサービス SDK を `requirements.txt` に追加
- `src/miqbot/bot.py` に実際のイベント処理を実装
- 必要に応じて環境変数 `.env` から設定を読み込むように変更

