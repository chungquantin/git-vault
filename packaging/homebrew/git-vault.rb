class GitVault < Formula
  desc "Manage multiple git identities and credentials per command, aws-vault style"
  homepage "https://github.com/chungquantin/git-vault"
  url "https://github.com/chungquantin/git-vault/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "08baec16425dc2f3f5bb816c3d1f95cf3cbbcc87d981ca6bdef921dc30cb6495"
  head "https://github.com/chungquantin/git-vault.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    (bash_completion/"git-vault").write Utils.safe_popen_read(bin/"git-vault", "completions", "bash")
    (zsh_completion/"_git-vault").write Utils.safe_popen_read(bin/"git-vault", "completions", "zsh")
    (fish_completion/"git-vault.fish").write Utils.safe_popen_read(bin/"git-vault", "completions", "fish")
  end

  def caveats
    <<~EOS
      Enable `git add vault` and `git <cmd> --vault <name>` by adding to ~/.zshrc:
        eval "$(git-vault shell-init)"
    EOS
  end

  test do
    ENV["GIT_VAULT_HOME"] = testpath/"vaults"
    assert_match "git-vault", shell_output("#{bin}/git-vault --version")
    system bin/"git-vault", "add", "--name", "t", "--user-name", "T", "--email", "t@example.com", "--no-token"
    assert_match "t@example.com", shell_output("#{bin}/git-vault list")
  end
end
