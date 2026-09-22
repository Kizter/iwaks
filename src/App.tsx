import "./App.css";
import iwaksMark from "./assets/iwaks-mark.png";

function App() {
  return (
    <main className="shell">
      <div className="brand">
        <img className="brand-mark" src={iwaksMark} alt="" />
        <h1>Iwaks</h1>
        <p>Hi-res &amp; lossless offline music player</p>
      </div>
    </main>
  );
}

export default App;