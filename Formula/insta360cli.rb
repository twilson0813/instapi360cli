class Insta360cli < Formula
  desc "List and download your own original Insta360 cloud media"
  homepage "https://github.com/twilson0813/instapi360cli"
  url "https://github.com/twilson0813/instapi360cli/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "be5ff163a728eef48dc93529e01e37af8de2c37a546b56072a946daaf0f818fb"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/twilson0813/instapi360cli.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/insta360cli-cli")
    bin.install "tools/organize_local.sh" => "insta360-organize"
    bin.install "tools/insta360cli-ui.sh" => "insta360cli-ui"
    bin.install "tools/rename_folders.sh" => "insta360-rename-folders"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/insta360cli --version")
    (testpath/"cam").mkpath
    (testpath/"cam/VID_20260809_131521_00_001.insv").write "x"
    (testpath/"cam/LRV_20260809_131521_01_001.lrv").write "x"
    system bin/"insta360-organize", testpath/"cam", testpath/"out"
    assert_path_exists testpath/"out/VID_20260809_131521/LRV_20260809_131521_01_001.lrv"
  end
end
