\NeedsTeXFormat{LaTeX2e}
\ProvidesClass{jcgt}[2012/06/21 Journal of Computer Graphics Techniques]

\LoadClass[11pt]{article}

\RequirePackage{graphicx}
\RequirePackage{fancyhdr}
\RequirePackage[labelfont=bf,textfont=it]{caption}
\RequirePackage{verbatim}
\RequirePackage{color}
\RequirePackage{listings}
\RequirePackage{amsfonts}
\RequirePackage{multirow}
\RequirePackage{amssymb,amsmath}
\RequirePackage{rotating}
\RequirePackage{pdfmarginpar}
\RequirePackage{natbib}
\pdfmarginparset{Open=false}

%\RequirePackage{stfloats} % Make position specifiers work with figure*
%\usepackage{fixltx2e} % prevent figure* from being placed out-of-order with respect to their "non-starred" counterparts

\newcommand{\jcgt@author}{???}
\newcommand{\jcgt@title}{???}
\newcommand{\jcgt@headtitle}{\jcgt@title}
\newcommand{\jcgt@submitted}{????-??-??}
\newcommand{\jcgt@recommended}{????-??-??}
\newcommand{\jcgt@published}{????-??-??}
\newcommand{\jcgt@editor}{??? ???}
\newcommand{\jcgt@volume}{?}
\newcommand{\jcgt@number}{?}
\newcommand{\jcgt@startpage}{1}
\newcommand{\jcgt@endpage}{1}
\newcommand{\jcgt@year}{????}
\newcommand{\jcgt@url}{????}

\newcommand{\setciteauthor}[1]{\renewcommand{\jcgt@author}{#1}}
\newcommand{\setcitetitle}[1]{\renewcommand{\jcgt@title}{#1}}
\newcommand{\setheadtitle}[1]{\renewcommand{\jcgt@headtitle}{#1}}

\newcommand{\seturl}[1]{\renewcommand{\jcgt@url}{#1}}

\newcommand{\submitted}[1]{
  \renewcommand{\jcgt@submitted}{#1}
  \lhead{\small Submitted to the \textit{Journal of Computer Graphics Techniques}}\chead{}\rhead{\small #1}
}

\newcommand{\accepted}[9]{%

\renewcommand{\jcgt@submitted}{#1}
\renewcommand{\jcgt@recommended}{#2}
\renewcommand{\jcgt@published}{#3}
\renewcommand{\jcgt@editor}{#4}
\renewcommand{\jcgt@volume}{#5}
\renewcommand{\jcgt@number}{#6}
\renewcommand{\jcgt@startpage}{#7}
\renewcommand{\jcgt@endpage}{#8}
\renewcommand{\jcgt@year}{#9}

% 
\lhead{\small \textit{Journal of Computer Graphics Techniques} \\ \jcgt@headtitle }\chead{}\rhead{\small Vol. \jcgt@volume, No. \jcgt@number, \jcgt@year \\\href{http://jcgt.org}{http://jcgt.org}}

\fancypagestyle{firstpagestyle}
{
   \fancyhead{\small \textit{Journal of Computer Graphics Techniques} Vol. \jcgt@volume, No. \jcgt@number, \jcgt@year}\chead{}\rhead{\small  \href{http://jcgt.org}{http://jcgt.org}}
   \fancyfoot[C]{\thepage }
   \fancyfoot[R]{ISSN~2331-7418}
}
}




\newcommand{\afterdoc}
{
\vspace{3mm}
\hrule
\vspace{3mm}
\vspace{5mm}
 \noindent\jcgt@author, \jcgt@title, \textit{Journal of Computer Graphics Techniques (JCGT)}, vol. \jcgt@volume, no. \jcgt@number, \jcgt@startpage--\jcgt@endpage, \jcgt@year\\\href{\jcgt@url}{\jcgt@url}

\vspace{5mm}
\noindent{\small\hspace{-3mm}
\begin{tabular*}{\textwidth}{lr@{\extracolsep{16mm}}l@{\extracolsep{3mm}}l}
Received: &\jcgt@submitted &  \\
Recommended: &\jcgt@recommended & Corresponding Editor: &\jcgt@editor\\
Published: &\jcgt@published & Editor-in-Chief: &Alexander Wilkie
\end{tabular*}}

\vspace{5mm}
\noindent \copyright~\jcgt@year~\jcgt@author~(the Authors).
\\The Authors provide this document (the Work) under the Creative Commons CC~BY-ND 4.0 license available online at \href{http://creativecommons.org/licenses/by-nd/4.0/}{http://creativecommons.org/licenses/by-nd/4.0/}. The Authors further grant permission for reuse of images and text from the first page of the Work, provided that the reuse is for the purpose of promoting and/or summarizing the Work in scholarly venues and that any reuse is accompanied by a scientific citation to the Work.

\vspace{2mm}
\noindent\includegraphics[width=2.5cm]{CC-BY-ND.png}
}



\newcommand{\transpose}{\top}
\usepackage{times}
\usepackage[scaled=.90]{helvet}
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% Listings

\lstset{
float, 
mathescape=true, 
texcl=true,
basicstyle=\footnotesize\ttfamily,
columns=fixed,
numbers=none,%left,
stepnumber=1,
captionpos=b,
showstringspaces=true,
language=C++,
keywordstyle=\color[rgb]{0.0, 0.0, 0.8},
stringstyle=\color[rgb]{0.5, 0.0, 0.5},
tabsize=4,
commentstyle=\color[rgb]{0.133,0.545,0.133}\textit,
morekeywords={ },
frame=single,
}

\definecolor{linkcolor}{rgb}{0,0.27,.52}
\usepackage[pdftex,
     pagebackref=true,
    colorlinks,linkcolor=linkcolor,citecolor=linkcolor,urlcolor=linkcolor,
    bookmarks=false,
    pdfpagemode=UseNone,
%    pdftitle={\jcgt@title},
%    pdfauthor={\jcgt@author},
%    pdfsubject={\pdf@Subject},
    pdfkeywords={Journal of Computer Graphics Techniques}]{hyperref}
%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% Figures
\setlength{\fboxsep}{0mm}

% Increase figures per page
\renewcommand\floatpagefraction{.8}
\renewcommand\topfraction{.99}
\renewcommand\bottomfraction{.99}
\renewcommand\textfraction{.01}   
\setcounter{totalnumber}{50}
\setcounter{topnumber}{50}
\setcounter{bottomnumber}{50}
\setcounter{topnumber}{10}
\setcounter{bottomnumber}{10}
\setcounter{totalnumber}{20}
\setcounter{dbltopnumber}{10}

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% Headers
\setlength{\headheight}{24pt}
\setlength{\headwidth}{31pc}
%\pagesytyle{fancy}
\pagestyle{fancyplain}
% Other headers inserted above
\lfoot{}\cfoot{\thepage}	\rfoot{}

  %%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% Page size
\setlength{\textwidth}{31pc}

\setlength{\topmargin}{-4mm}
\setlength{\textheight}{21.2cm}

% Line spacing
\renewcommand{\baselinestretch}{1.1} 

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% Bibliography

% [Author year; author year]
\bibpunct{[}{]}{;}{a}{}{,}

% make bare \cite default to \citep
\renewcommand{\cite}[1]{\citep{#1}}

\def\@biblabel#1{}

% set the first line of each reference flush left, and each succeeding
% line with a 1em indentation.

\setlength{\bibhang}{1em}


%%%\def\thebibliography#1{\par\footnotesize
\def\thebibliography#1{%
  \section*{%
    \refname\@mkboth{\sl\uppercase{\refname}}{\sl\uppercase{\refname}}}
  \list{\relax}{\setlength{\labelsep}{0em}
                \setlength{\itemindent}{-\bibhang}
                \setlength{\leftmargin}{\bibhang}}
  \def\newblock{\hskip .11em plus .33em minus .07em}
  \sloppy\clubpenalty4000\widowpenalty4000
  \sfcode`\.=1000\relax}


\def\@citex[#1]#2{\if@filesw\immediate\write\@auxout{\string\citation{#2}}\fi
  \def\@citea{}\@cite{\@for\@citeb:=#2\do
    {\@citea\def\@citea{; }\@ifundefined
      {b@\@citeb}{{\bf ?}\@warning
      {Citation '\@citeb' on page \thepage \space undefined}}%
{\csname b@\@citeb\endcsname}}}{#1}}

% Teaser image
\newcommand{\jcgt@teaser}{}
\newcommand{\teaser}[1]{\renewcommand{\jcgt@teaser}{#1}}


% Title formatting
\renewcommand{\@maketitle}{%
\hypersetup{pdftitle={\jcgt@title},pdfauthor={\jcgt@author}}
\setcounter{page}{\jcgt@startpage}
\thispagestyle{fancy}
        \begin{center}%
            \renewcommand{\thanks}[1]{}
            \vspace*{1pc}
            {\LARGE\sffamily\bfseries  \@title \par}%
            \vspace{1.5\baselineskip}%
	    \begin{tabular}[t]{c}%
                \@author
            \end{tabular}\par%      
              \vspace{1\baselineskip}%

            \ifx\jcgt@teaser {} 
            \else%
                \vspace{0.25in}%
                \begingroup%
                    \def\@captype{figure}%
                    \jcgt@teaser%
                \endgroup\par%
                \vspace{0.2in}%
            \fi%       
        \end{center} \par
        }

% Abstract formatting
\renewenvironment{abstract}%
            {\subsection*{Abstract}}
            {}

% Section heading formatting

\newif\iffirstsec
\firstsecfalse
\renewcommand\section{\firstsectrue\@startsection {section}{1}{\z@}%
                {-1.0ex}%
                {1.0ex}%
                {\vspace{0.2in}\reset@font\large\sffamily\bfseries}}

\renewcommand\subsection{\@startsection{subsection}{2}{\z@}%
                {-1.0ex}%
                {1.0ex}%
                {\vspace{0.1in}\reset@font\large\sffamily}}
                
\renewcommand\subsubsection{\@startsection{subsubsection}{3}{\z@}%
                {-1.0ex}%
                {1.0ex}%
                {\reset@font\normalsize\sffamily\itshape}}



\renewcommand\paragraph{\@startsection{paragraph}{4}{\z@}%
                {2.05ex}%
                {-1em}%
                {\reset@font\normalsize\sffamily\itshape}}
                
\renewcommand\subparagraph{\@startsection{subparagraph}{5}{\parindent}%
                {3.25ex \@plus1ex \@minus .2ex}%
                {-1em}%
                {\reset@font\normalsize\sffamily\bfseries}}
                                  
% Add trailing period to section heads greater than one but not in references
\renewcommand{\@seccntformat}[1]{\iffirstsec{\@nameuse{the#1}}.\hspace{1pc}\else{\@nameuse{the#1}}\hspace{1pc}\fi}



\long\def\@makecaption#1#2{%
  \vskip\abovecaptionskip
  \sbox\@tempboxa{\small  \sffamily {\bfseries #1}. \small #2}%
  \ifdim \wd\@tempboxa >\hsize
%{ \small  \sffamily \bfseries  #1}. \small \sffamily #2\par
\centering{\setlength{\baselineskip}{11.5pt}
\begin{minipage}{\linewidth}
 {\small  \sffamily {\bfseries  #1}. \small  #2}\par
\end{minipage}}
  \else
    \global \@minipagefalse
    \hb@xt@\hsize{\hfil\box\@tempboxa\hfil}%
  \fi
  \vskip\belowcaptionskip}
  
  

